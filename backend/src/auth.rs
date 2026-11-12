//! Authentication: password hashing, session cookies, CSRF protection, login rate limiting and
//! the `AuthUser` / `AdminUser` extractors.
//!
//! Sessions are signed JWTs stored in an **HttpOnly** cookie (`hw_session`), so page scripts
//! can't read them. Each session carries a random CSRF token. The token is also sent as a
//! readable cookie (`hw_csrf`), and the frontend must echo it in the `X-CSRF-Token` header on
//! every state-changing request. Both cookies are `SameSite=Strict`.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, HeaderMap, HeaderValue, Method},
};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{
    args,
    config::Config,
    db,
    error::{AppError, AppResult},
    models::User,
    AppState,
};

pub const MIN_PASSWORD_LEN: usize = 8;
pub const SESSION_COOKIE: &str = "hw_session";
pub const CSRF_COOKIE: &str = "hw_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub role: String,
    /// Must equal `users.session_epoch`; bumping the epoch revokes every existing session.
    pub ver: i64,
    /// CSRF token bound to this session.
    pub csrf: String,
    pub iat: usize,
    pub exp: usize,
}

fn hash_blocking(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(e.to_string()))
}

fn verify_blocking(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Hashes without a concurrency limit. Request handlers use [`HashLimiter::hash`] instead; this is
/// for startup (seeding the admin) and tests.
pub async fn hash_password(password: String) -> AppResult<String> {
    // Argon2 is deliberately CPU heavy: keep it off the async executor.
    tokio::task::spawn_blocking(move || hash_blocking(&password))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

/// Verifies without a concurrency limit (see [`hash_password`]).
pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_blocking(&password, &hash))
        .await
        .unwrap_or(false)
}

/// Caps how many Argon2 operations (each uses ~19 MiB of memory and a CPU core for a while) run
/// at once, so a flood of logins can't exhaust the server. A request that can't get a slot within
/// `wait` fails with [`AppError::Busy`] (503) instead of queueing without limit.
pub struct HashLimiter {
    slots: Arc<Semaphore>,
    wait: Duration,
}

impl HashLimiter {
    pub fn new(max_concurrent: usize, wait: Duration) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(max_concurrent.max(1))),
            wait,
        }
    }

    /// Runs `work` on the blocking thread pool while holding a slot. The slot is released when
    /// `work` finishes, even if the request was cancelled in the meantime.
    pub async fn run<T, F>(&self, work: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = match tokio::time::timeout(self.wait, self.slots.clone().acquire_owned()).await
        {
            Ok(Ok(permit)) => permit,
            _ => {
                tracing::warn!("password hashing is at capacity; request refused with 503");
                return Err(AppError::Busy);
            }
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
    }

    pub async fn hash(&self, password: String) -> AppResult<String> {
        self.run(move || hash_blocking(&password)).await?
    }

    pub async fn verify(&self, password: String, hash: String) -> AppResult<bool> {
        self.run(move || verify_blocking(&password, &hash)).await
    }
}

/// A valid Argon2 hash of a random string. Verifying against it when an email is unknown keeps
/// the response time the same as for a wrong password, so login can't be used to discover which
/// emails have accounts.
pub const DUMMY_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$5b3OB7W0nNQ1G1aXGyYwGKXrqt6R3v1tWnvy2F7kE0o";

pub fn validate_password(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AppError::bad(format!(
            "Password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if password.chars().count() > 256 {
        return Err(AppError::bad("Password must be at most 256 characters"));
    }
    Ok(())
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time string comparison (for CSRF tokens).
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

pub fn create_token(user: &User, csrf: &str, secret: &str, ttl_hours: i64) -> AppResult<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user.id,
        role: user.role.clone(),
        ver: user.session_epoch,
        csrf: csrf.to_string(),
        iat: now.timestamp() as usize,
        exp: (now + chrono::Duration::hours(ttl_hours)).timestamp() as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn decode_token(token: &str, secret: &str) -> Option<Claims> {
    let mut validation = Validation::default();
    validation.leeway = 0;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

// ---------------------------------------------------------------- cookies

fn cookie_attrs(config: &Config) -> &'static str {
    if config.cookie_secure {
        "; Path=/; SameSite=Strict; Secure"
    } else {
        "; Path=/; SameSite=Strict"
    }
}

/// `Set-Cookie` headers that start a session for `user`.
pub fn session_cookies(user: &User, config: &Config) -> AppResult<Vec<HeaderValue>> {
    let csrf = random_token();
    let token = create_token(user, &csrf, &config.jwt_secret, config.jwt_ttl_hours)?;
    let max_age = config.jwt_ttl_hours * 3600;
    let attrs = cookie_attrs(config);
    [
        format!("{SESSION_COOKIE}={token}; Max-Age={max_age}; HttpOnly{attrs}"),
        format!("{CSRF_COOKIE}={csrf}; Max-Age={max_age}{attrs}"),
    ]
    .into_iter()
    .map(|c| HeaderValue::from_str(&c).map_err(|e| AppError::Internal(e.to_string())))
    .collect()
}

/// `Set-Cookie` headers that end the session.
pub fn clear_cookies(config: &Config) -> Vec<HeaderValue> {
    let attrs = cookie_attrs(config);
    [
        format!("{SESSION_COOKIE}=; Max-Age=0; HttpOnly{attrs}"),
        format!("{CSRF_COOKIE}=; Max-Age=0{attrs}"),
    ]
    .into_iter()
    .filter_map(|c| HeaderValue::from_str(&c).ok())
    .collect()
}

pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------- rate limiting

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum LimitKey {
    /// One email from one client address.
    Account(String, IpAddr),
    /// One client address, across all emails (password spraying).
    Ip(IpAddr),
}

/// In-memory limiter for failed logins. It has two counters, both over the same `window`:
///
/// - **per email and client IP** (`max_per_account`). One client can't keep guessing a
///   password, but it also can't lock the account for anyone else: the real user signs in from
///   another address. There is deliberately no per-email-only lock, because that would let
///   anyone lock out any known email.
/// - **per client IP** (`max_per_ip`, across all emails). This stops one client from spraying a
///   few common passwords over many accounts.
///
/// Every attempt is counted when it **starts** ([`LoginLimiter::begin`]), so many simultaneous
/// requests can't all pass the check before the first failure is recorded. A successful login
/// refunds its attempt and clears that email+IP counter. An attempt that never reached the
/// password check (server busy, database error) is refunded too.
///
/// The state is per process: run one backend replica (see docs/DEPLOYMENT.md).
pub struct LoginLimiter {
    max_per_account: u32,
    max_per_ip: u32,
    window: Duration,
    entries: Mutex<HashMap<LimitKey, (u32, Instant)>>,
}

impl LoginLimiter {
    pub fn new(max_per_account: u32, max_per_ip: u32, window: Duration) -> Self {
        Self {
            max_per_account: max_per_account.max(1),
            max_per_ip: max_per_ip.max(1),
            window,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn keys(email: &str, ip: IpAddr) -> [(LimitKey, bool); 2] {
        [
            (LimitKey::Ip(ip), true),
            (LimitKey::Account(email.to_string(), ip), false),
        ]
    }

    fn max_for(&self, per_ip: bool) -> u32 {
        if per_ip {
            self.max_per_ip
        } else {
            self.max_per_account
        }
    }

    /// Reserves one attempt for `email` from `ip`, or `Err(seconds_remaining)` while either
    /// counter is at its limit. The returned guard refunds the attempt unless
    /// [`LoginAttempt::failed`] is called.
    pub fn begin(&self, email: &str, ip: IpAddr) -> Result<LoginAttempt<'_>, u64> {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        map.retain(|_, (_, start)| now.duration_since(*start) < self.window);
        let keys = Self::keys(email, ip);
        let locked = keys
            .iter()
            .filter_map(|(key, per_ip)| {
                map.get(key)
                    .filter(|(count, _)| *count >= self.max_for(*per_ip))
                    .map(|(_, start)| self.window.saturating_sub(now.duration_since(*start)))
            })
            .max();
        if let Some(left) = locked {
            return Err(left.as_secs().max(1));
        }
        for (key, _) in keys {
            map.entry(key).or_insert((0, now)).0 += 1;
        }
        Ok(LoginAttempt {
            limiter: self,
            email: email.to_string(),
            ip,
            outcome: Outcome::Refund,
        })
    }

    fn refund(&self, email: &str, ip: IpAddr, clear_account: bool) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        for (key, per_ip) in Self::keys(email, ip) {
            if !per_ip && clear_account {
                map.remove(&key);
            } else if let Some(entry) = map.get_mut(&key) {
                entry.0 = entry.0.saturating_sub(1);
            }
        }
    }
}

enum Outcome {
    Refund,
    Failed,
    Succeeded,
}

/// One reserved login attempt (see [`LoginLimiter::begin`]).
pub struct LoginAttempt<'a> {
    limiter: &'a LoginLimiter,
    email: String,
    ip: IpAddr,
    outcome: Outcome,
}

impl LoginAttempt<'_> {
    /// Wrong email or password: the attempt stays counted.
    pub fn failed(mut self) {
        self.outcome = Outcome::Failed;
    }

    /// Correct password: the attempt is refunded and the email+IP counter cleared.
    pub fn succeeded(mut self) {
        self.outcome = Outcome::Succeeded;
    }
}

impl Drop for LoginAttempt<'_> {
    fn drop(&mut self) {
        match self.outcome {
            Outcome::Failed => {}
            Outcome::Refund => self.limiter.refund(&self.email, self.ip, false),
            Outcome::Succeeded => self.limiter.refund(&self.email, self.ip, true),
        }
    }
}

// ---------------------------------------------------------------- extractors

fn is_safe_method(m: &Method) -> bool {
    matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Any signed-in, active user. The user row is re-read on every request so role changes,
/// deactivation and revoked sessions take effect immediately. State-changing requests must
/// carry the session's CSRF token in `X-CSRF-Token`.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = read_cookie(&parts.headers, SESSION_COOKIE).ok_or(AppError::Unauthorized)?;
        let claims =
            decode_token(&token, &state.config.jwt_secret).ok_or(AppError::Unauthorized)?;

        if !is_safe_method(&parts.method) {
            let sent = parts
                .headers
                .get(CSRF_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !constant_time_eq(sent, &claims.csrf) {
                return Err(AppError::Forbidden);
            }
        }

        let mut conn = state.db.conn().await?;
        let user: User = db::query_opt(
            &mut conn,
            "SELECT * FROM users WHERE id = $1",
            args![claims.sub],
        )
        .await?
        .ok_or(AppError::Unauthorized)?;

        if !user.is_active || user.session_epoch != claims.ver {
            return Err(AppError::Unauthorized);
        }
        Ok(AuthUser(user))
    }
}

/// A signed-in user with the admin role.
pub struct AdminUser(pub User);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err(AppError::Forbidden);
        }
        Ok(AdminUser(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> User {
        User {
            id: Uuid::new_v4(),
            email: "a@b.co".into(),
            name: "A".into(),
            password_hash: String::new(),
            role: "member".into(),
            job_title: None,
            avatar_url: None,
            is_active: true,
            session_epoch: 3,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    #[tokio::test]
    async fn password_hash_roundtrip() {
        let hash = hash_password("correct horse".into())
            .await
            .unwrap_or_default();
        assert!(hash.starts_with("$argon2id$"));
        assert!(!hash.contains("correct horse"));
        assert!(verify_password("correct horse".into(), hash.clone()).await);
        assert!(!verify_password("Correct horse".into(), hash.clone()).await);
        assert!(!verify_password(String::new(), hash).await);
        assert!(!verify_password("x".into(), "not-a-hash".into()).await);
        // Two hashes of the same password differ (random salt).
        let a = hash_password("same".into()).await.unwrap_or_default();
        let b = hash_password("same".into()).await.unwrap_or_default();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn dummy_hash_is_valid_but_matches_nothing_common() {
        assert!(PasswordHash::new(DUMMY_HASH).is_ok());
        assert!(!verify_password("password".into(), DUMMY_HASH.into()).await);
    }

    #[test]
    fn password_rules() {
        assert!(validate_password("1234567").is_err());
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password(&"x".repeat(257)).is_err());
    }

    #[test]
    fn tokens_roundtrip_and_reject_tampering() {
        let u = user();
        let t = create_token(&u, "csrf", SECRET, 1).unwrap_or_default();
        let c = decode_token(&t, SECRET);
        assert_eq!(c.as_ref().map(|c| c.sub), Some(u.id));
        assert_eq!(c.as_ref().map(|c| c.ver), Some(3));
        assert!(decode_token(&t, "another-secret-another-secret-xx").is_none());
        assert!(decode_token(&format!("{t}x"), SECRET).is_none());
        assert!(decode_token("garbage", SECRET).is_none());
    }

    #[test]
    fn expired_tokens_are_rejected() {
        let t = create_token(&user(), "csrf", SECRET, -1).unwrap_or_default();
        assert!(decode_token(&t, SECRET).is_none());
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; hw_session=tok; hw_csrf=c"),
        );
        assert_eq!(read_cookie(&h, "hw_session").as_deref(), Some("tok"));
        assert_eq!(read_cookie(&h, "hw_csrf").as_deref(), Some("c"));
        assert_eq!(read_cookie(&h, "missing"), None);
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(!constant_time_eq("", "a"));
    }

    fn ip(s: &str) -> IpAddr {
        s.parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    }

    fn fail(l: &LoginLimiter, email: &str, from: &str) -> bool {
        l.begin(email, ip(from)).map(LoginAttempt::failed).is_ok()
    }

    #[test]
    fn limiter_locks_an_email_only_for_the_failing_address() {
        let l = LoginLimiter::new(3, 100, Duration::from_secs(60));
        for _ in 0..3 {
            assert!(fail(&l, "victim", "198.51.100.1"));
        }
        assert!(l.begin("victim", ip("198.51.100.1")).is_err());
        // The real user, from another address, is not locked out.
        assert!(l.begin("victim", ip("203.0.113.5")).is_ok());
        // The attacker can still try other emails until the per-IP limit.
        assert!(l.begin("other", ip("198.51.100.1")).is_ok());
    }

    #[test]
    fn limiter_blocks_password_spraying_per_address() {
        let l = LoginLimiter::new(3, 5, Duration::from_secs(60));
        for i in 0..5 {
            assert!(fail(&l, &format!("user{i}"), "198.51.100.1"));
        }
        let left = l.begin("user99", ip("198.51.100.1")).err();
        assert!(left.is_some_and(|s| (1..=60).contains(&s)));
        assert!(l.begin("user99", ip("203.0.113.5")).is_ok());
    }

    #[test]
    fn limiter_success_and_refunds() {
        let l = LoginLimiter::new(3, 4, Duration::from_secs(60));
        assert!(fail(&l, "x", "198.51.100.1"));
        assert!(fail(&l, "x", "198.51.100.1"));
        // Success clears the email+IP counter and doesn't count against the address.
        if let Ok(a) = l.begin("x", ip("198.51.100.1")) {
            a.succeeded();
        }
        for _ in 0..2 {
            assert!(fail(&l, "x", "198.51.100.1"));
        }
        // Per IP: 4 failures so far, so the address is now blocked.
        assert!(l.begin("y", ip("198.51.100.1")).is_err());

        // An attempt dropped without an outcome (server busy, database error) is refunded.
        let l = LoginLimiter::new(1, 1, Duration::from_secs(60));
        for _ in 0..5 {
            drop(l.begin("x", ip("198.51.100.1")));
        }
        assert!(l.begin("x", ip("198.51.100.1")).is_ok());
    }

    #[test]
    fn limiter_counts_attempts_while_they_are_in_flight() {
        let l = LoginLimiter::new(2, 100, Duration::from_secs(60));
        let a = l.begin("x", ip("198.51.100.1"));
        let b = l.begin("x", ip("198.51.100.1"));
        assert!(a.is_ok() && b.is_ok());
        // Two attempts are still being checked: a third concurrent one is refused.
        assert!(l.begin("x", ip("198.51.100.1")).is_err());
        drop((a, b));
        assert!(l.begin("x", ip("198.51.100.1")).is_ok());
    }

    #[test]
    fn limiter_window_expires() {
        let l = LoginLimiter::new(1, 1, Duration::from_millis(30));
        assert!(fail(&l, "x", "198.51.100.1"));
        assert!(l.begin("x", ip("198.51.100.1")).is_err());
        std::thread::sleep(Duration::from_millis(40));
        assert!(l.begin("x", ip("198.51.100.1")).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn hash_limiter_caps_concurrency() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let limiter = Arc::new(HashLimiter::new(4, Duration::from_secs(30)));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let (limiter, running, peak) = (limiter.clone(), running.clone(), peak.clone());
                tokio::spawn(async move {
                    limiter
                        .run(move || {
                            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(40));
                            running.fetch_sub(1, Ordering::SeqCst);
                        })
                        .await
                        .is_ok()
                })
            })
            .collect();
        let mut done = 0;
        for t in tasks {
            done += usize::from(t.await.unwrap_or(false));
        }
        assert_eq!(done, 16, "every job eventually got a slot");
        assert_eq!(peak.load(Ordering::SeqCst), 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hash_limiter_refuses_when_saturated_and_recovers() {
        let limiter = Arc::new(HashLimiter::new(1, Duration::from_millis(50)));
        let busy = limiter.clone();
        let slow = tokio::spawn(async move {
            busy.run(|| std::thread::sleep(Duration::from_millis(400)))
                .await
                .is_ok()
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(matches!(limiter.run(|| ()).await, Err(AppError::Busy)));
        assert!(slow.await.unwrap_or(false));
        // Once the slot is free again, work runs normally.
        let hash = limiter
            .hash("correct horse".into())
            .await
            .unwrap_or_default();
        assert_eq!(
            limiter.verify("correct horse".into(), hash).await.ok(),
            Some(true)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hash_limiter_holds_the_slot_after_the_caller_is_cancelled() {
        let limiter = Arc::new(HashLimiter::new(1, Duration::from_millis(50)));
        let cancelled = limiter.clone();
        let task = tokio::spawn(async move {
            let _ = cancelled
                .run(|| std::thread::sleep(Duration::from_millis(400)))
                .await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The client went away, but the hash is still running: the slot is still taken.
        task.abort();
        let _ = task.await;
        assert!(matches!(limiter.run(|| ()).await, Err(AppError::Busy)));
    }

    #[test]
    fn random_tokens_are_unique_hex() {
        let a = random_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_token());
    }
}
