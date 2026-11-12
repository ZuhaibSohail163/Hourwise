//! The client's IP address, for login rate limiting.
//!
//! By default the address is the TCP peer, and `X-Forwarded-For` is **ignored**, because any
//! client can send that header. Proxy headers are only used when the peer is inside one of the
//! ranges in `TRUSTED_PROXIES`. The header is then read from right to left, skipping trusted
//! addresses, and the first untrusted address is the client. Entries further left were written
//! by the client (or by a hop we can't verify) and are never used.
//!
//! In production the chain is browser → Traefik → Next.js frontend → backend. The Next.js
//! proxy passes `X-Forwarded-For` through unchanged without adding an entry, and Traefik adds
//! the address that connected to it as the rightmost entry. So with the frontend's private network
//! in `TRUSTED_PROXIES`, the rightmost entry is the address Traefik saw. See docs/DEPLOYMENT.md.

use std::{
    convert::Infallible,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::atomic::{AtomicBool, Ordering},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{request::Parts, HeaderMap},
};

use crate::AppState;

const FORWARDED_FOR: &str = "x-forwarded-for";

/// An IP address or CIDR range, e.g. `10.0.1.0/24`, `172.18.0.5` or `fd00::/8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        let (ip, prefix) = match value.split_once('/') {
            Some((ip, prefix)) => (ip, Some(prefix)),
            None => (value, None),
        };
        let addr = ip
            .parse::<IpAddr>()
            .map_err(|_| format!("`{value}` is not an IP address or CIDR range"))?
            .to_canonical();
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("`{value}` has an invalid prefix length"))?,
        };
        Ok(Self { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for IpNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// Parses a comma-separated `TRUSTED_PROXIES` value. Empty means "trust no proxy".
pub fn parse_trusted_proxies(value: &str) -> Result<Vec<IpNet>, String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(IpNet::parse)
        .collect()
}

fn parse_hop(hop: &str) -> Option<IpAddr> {
    hop.parse::<IpAddr>()
        .ok()
        .or_else(|| hop.parse::<SocketAddr>().ok().map(|a| a.ip()))
        .map(|ip| ip.to_canonical())
}

/// The client address for a request that arrived from `peer`.
pub fn resolve(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let peer = peer.to_canonical();
    let is_trusted = |ip: IpAddr| trusted.iter().any(|net| net.contains(ip));
    if !is_trusted(peer) {
        return peer;
    }
    let hops: Vec<&str> = headers
        .get_all(FORWARDED_FOR)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .collect();
    let mut client = peer;
    for hop in hops.iter().rev() {
        match parse_hop(hop) {
            Some(ip) if !is_trusted(ip) => return ip,
            Some(ip) => client = ip,
            // A malformed entry: stop at the last address a trusted hop vouched for.
            None => return client,
        }
    }
    client
}

static WARNED_UNTRUSTED: AtomicBool = AtomicBool::new(false);

/// Extractor for the client IP. The TCP peer comes from `ConnectInfo`, which `main.rs` installs;
/// without it (e.g. in-process tests) the peer is `0.0.0.0`.
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0.ip());
        let trusted = &state.config.trusted_proxies;
        let ip = resolve(peer, &parts.headers, trusted);
        if ip == peer.to_canonical()
            && parts.headers.contains_key(FORWARDED_FOR)
            && !trusted.iter().any(|net| net.contains(peer))
            && !WARNED_UNTRUSTED.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(
                "ignoring X-Forwarded-For from {peer}, which is not in TRUSTED_PROXIES: login rate \
                 limits use the connecting address. Behind the Dokploy frontend, set TRUSTED_PROXIES \
                 to the frontend's network (docs/DEPLOYMENT.md). Logged once."
            );
        }
        Ok(ClientIp(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    }

    fn nets(s: &str) -> Vec<IpNet> {
        parse_trusted_proxies(s).unwrap_or_default()
    }

    fn xff(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            if let Ok(v) = HeaderValue::from_str(v) {
                h.append(FORWARDED_FOR, v);
            }
        }
        h
    }

    #[test]
    fn parses_addresses_and_ranges() {
        assert_eq!(nets("10.0.1.0/24, 172.18.0.5 ,fd00::/8").len(), 3);
        assert!(nets("").is_empty());
        assert!(nets(" , ").is_empty());
        assert!(parse_trusted_proxies("10.0.0.0/33").is_err());
        assert!(parse_trusted_proxies("fd00::/129").is_err());
        assert!(parse_trusted_proxies("10.0.0.0/x").is_err());
        assert!(parse_trusted_proxies("not-an-ip").is_err());
        assert!(parse_trusted_proxies("example.com").is_err());
    }

    #[test]
    fn range_membership() {
        let net = nets("10.0.1.0/24")[0];
        assert!(net.contains(ip("10.0.1.0")));
        assert!(net.contains(ip("10.0.1.255")));
        assert!(!net.contains(ip("10.0.2.1")));
        assert!(!net.contains(ip("::1")));
        // IPv4-mapped IPv6 (as some stacks report peers) matches the IPv4 range.
        assert!(net.contains(ip("::ffff:10.0.1.7")));
        assert!(nets("0.0.0.0/0")[0].contains(ip("8.8.8.8")));
        assert!(nets("172.18.0.5")[0].contains(ip("172.18.0.5")));
        assert!(!nets("172.18.0.5")[0].contains(ip("172.18.0.6")));
        assert!(nets("fd00::/8")[0].contains(ip("fd12::1")));
        assert!(!nets("fd00::/8")[0].contains(ip("fe80::1")));
    }

    #[test]
    fn header_is_ignored_from_untrusted_peers() {
        let h = xff(&["6.6.6.6"]);
        assert_eq!(resolve(ip("172.18.0.3"), &h, &[]), ip("172.18.0.3"));
        assert_eq!(
            resolve(ip("203.0.113.9"), &h, &nets("10.0.1.0/24")),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn trusted_peer_uses_the_rightmost_untrusted_entry() {
        let trusted = nets("10.0.1.0/24");
        let peer = ip("10.0.1.4");
        // Traefik's entry is the rightmost one; anything the client wrote is to its left.
        let h = xff(&["6.6.6.6, 198.51.100.7"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("198.51.100.7"));
        // Several headers are read in order, as one list.
        let h = xff(&["6.6.6.6", "198.51.100.7"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("198.51.100.7"));
        // Trusted hops (e.g. another proxy in the range) are skipped.
        let h = xff(&["198.51.100.7, 10.0.1.9"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("198.51.100.7"));
        // Ports and IPv6 forms are understood.
        let h = xff(&["198.51.100.7:4711"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("198.51.100.7"));
        let h = xff(&["[2001:db8::1]:443"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("2001:db8::1"));
    }

    #[test]
    fn missing_or_malformed_headers_fall_back_to_trusted_hops() {
        let trusted = nets("10.0.1.0/24");
        let peer = ip("10.0.1.4");
        assert_eq!(resolve(peer, &HeaderMap::new(), &trusted), peer);
        // A garbage entry can't be used to pick an arbitrary identity.
        let h = xff(&["6.6.6.6, garbage"]);
        assert_eq!(resolve(peer, &h, &trusted), peer);
        let h = xff(&["6.6.6.6, garbage, 10.0.1.9"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("10.0.1.9"));
        // Only trusted entries: the leftmost trusted one.
        let h = xff(&["10.0.1.8, 10.0.1.9"]);
        assert_eq!(resolve(peer, &h, &trusted), ip("10.0.1.8"));
    }
}
