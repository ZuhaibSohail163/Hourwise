//! Authentication: password hashing, session cookies, CSRF protection, login rate limiting and
//! the `AuthUser` / `AdminUser` extractors.
//!
//! Sessions are signed JWTs stored in an **HttpOnly** cookie (`hw_session`), so page scripts
//! can't read them. Each session carries a random CSRF token. The token is also sent as a
//! readable cookie (`hw_csrf`), and the frontend must echo it in the `X-CSRF-Token` header on
//! every state-changing request. Both cookies are `SameSite=Strict`.

use std::{
    collections::HashMap,
    sync::Mutex,
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

pub async fn hash_password(password: String) -> AppResult<String> {
    // Argon2 is deliberately CPU heavy: keep it off the async executor.
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AppError::Internal(e.to_string()))
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
}

pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
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

/// In-memory limiter for failed logins, keyed by email. After `max_failures` failures within
/// `window`, further attempts for that email are refused until the window expires. A successful
/// login clears the counter. (Single-instance deployments; see README for multi-instance notes.)
pub struct LoginLimiter {
    max_failures: u32,
    window: Duration,
    entries: Mutex<HashMap<String, (u32, Instant)>>,
}

impl LoginLimiter {
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures: max_failures.max(1),
            window,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// `Err(seconds_remaining)` while the key is locked out.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        map.retain(|_, (_, start)| now.duration_since(*start) < self.window);
        match map.get(key) {
            Some((count, start)) if *count >= self.max_failures => {
                let left = self.window.saturating_sub(now.duration_since(*start));
                Err(left.as_secs().max(1))
            }
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self, key: &str) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_insert((0, now));
        if now.duration_since(entry.1) >= self.window {
            *entry = (0, now);
        }
        entry.0 += 1;
    }

    pub fn reset(&self, key: &str) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(key);
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

        let conn = state.db.conn().await?;
        let user: User = db::query_opt(
            &conn,
            "SELECT * FROM users WHERE id = ?1",
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

    #[test]
    fn limiter_locks_after_max_failures_and_resets() {
        let l = LoginLimiter::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            assert!(l.check("x").is_ok());
            l.record_failure("x");
        }
        assert!(l.check("x").is_err());
        assert!(l.check("other").is_ok());
        l.reset("x");
        assert!(l.check("x").is_ok());
    }

    #[test]
    fn limiter_window_expires() {
        let l = LoginLimiter::new(1, Duration::from_millis(30));
        l.record_failure("x");
        assert!(l.check("x").is_err());
        std::thread::sleep(Duration::from_millis(40));
        assert!(l.check("x").is_ok());
    }

    #[test]
    fn random_tokens_are_unique_hex() {
        let a = random_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, random_token());
    }
}
