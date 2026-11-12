use std::{env, fmt, time::Duration};

use crate::{
    client_ip::{parse_trusted_proxies, IpNet},
    db::DbSettings,
};

#[derive(Clone)]
pub struct Config {
    pub database: DbSettings,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub jwt_ttl_hours: i64,
    pub upload_dir: String,
    pub cors_origin: Option<String>,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub admin_name: String,
    /// Adds `Secure` to session cookies (HTTPS only). Browsers also accept Secure cookies on
    /// http://localhost, so this stays on for local Docker use.
    pub cookie_secure: bool,
    /// Failed logins allowed per email **from one client IP** before that pair is locked for
    /// `login_lockout_secs`.
    pub login_max_attempts: u32,
    /// Failed logins allowed per client IP across all emails (password spraying).
    pub login_max_attempts_per_ip: u32,
    pub login_lockout_secs: u64,
    /// Peers whose `X-Forwarded-For` is believed (see `client_ip`). Empty: trust no proxy.
    pub trusted_proxies: Vec<IpNet>,
    /// Argon2 operations allowed at the same time (see `auth::HashLimiter`).
    pub password_hash_concurrency: usize,
}

/// Hand-written so secrets never end up in logs.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("database", &self.database)
            .field("bind_addr", &self.bind_addr)
            .field("jwt_secret", &"<redacted>")
            .field("jwt_ttl_hours", &self.jwt_ttl_hours)
            .field("upload_dir", &self.upload_dir)
            .field("cors_origin", &self.cors_origin)
            .field("admin_email", &self.admin_email)
            .field(
                "admin_password",
                &self.admin_password.as_ref().map(|_| "<redacted>"),
            )
            .field("cookie_secure", &self.cookie_secure)
            .field("login_max_attempts", &self.login_max_attempts)
            .field("login_max_attempts_per_ip", &self.login_max_attempts_per_ip)
            .field("login_lockout_secs", &self.login_lockout_secs)
            .field(
                "trusted_proxies",
                &self
                    .trusted_proxies
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
            .field("password_hash_concurrency", &self.password_hash_concurrency)
            .finish()
    }
}

fn opt(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Config {
    /// Reads configuration from the environment. Panics with a clear message when required
    /// settings are missing (the process cannot do anything useful without them).
    pub fn from_env() -> Self {
        let jwt_secret = opt("JWT_SECRET").expect("JWT_SECRET must be set");
        if jwt_secret.len() < 32 {
            panic!("JWT_SECRET must be at least 32 characters long");
        }
        Self {
            database: database_from_env().unwrap_or_else(|e| panic!("{e}")),
            bind_addr: opt("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()),
            jwt_secret,
            jwt_ttl_hours: opt("JWT_TTL_HOURS")
                .and_then(|v| v.parse().ok())
                .filter(|h| *h > 0)
                .unwrap_or(24 * 7),
            upload_dir: opt("UPLOAD_DIR").unwrap_or_else(|| "./uploads".into()),
            cors_origin: opt("CORS_ORIGIN"),
            admin_email: opt("ADMIN_EMAIL").map(|e| e.to_lowercase()),
            admin_password: opt("ADMIN_PASSWORD"),
            admin_name: opt("ADMIN_NAME").unwrap_or_else(|| "Administrator".into()),
            cookie_secure: opt("COOKIE_SECURE").is_none_or(|v| {
                !matches!(
                    v.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            }),
            login_max_attempts: opt("LOGIN_MAX_ATTEMPTS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            login_max_attempts_per_ip: opt_num("LOGIN_MAX_ATTEMPTS_PER_IP").unwrap_or(20),
            login_lockout_secs: opt("LOGIN_LOCKOUT_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(15 * 60),
            trusted_proxies: parse_trusted_proxies(&opt("TRUSTED_PROXIES").unwrap_or_default())
                .unwrap_or_else(|e| panic!("TRUSTED_PROXIES: {e}")),
            password_hash_concurrency: opt_num::<usize>("PASSWORD_HASH_CONCURRENCY")
                .filter(|n| *n > 0)
                .unwrap_or(4),
        }
    }
}

fn opt_num<T: std::str::FromStr>(key: &str) -> Option<T> {
    opt(key).and_then(|v| v.parse().ok())
}

/// Database settings only (used by the `migrate` / `check-db` / `verify` commands, which
/// don't need the JWT secret).
///
/// - `DATABASE_URL` (required): `postgres://user:password@host:5432/dbname`
/// - `DATABASE_MAX_CONNECTIONS` (default 10): pool size per backend instance
/// - `DATABASE_ACQUIRE_TIMEOUT_SECS` (default 5): wait for a free connection, then 503
/// - `DATABASE_STATEMENT_TIMEOUT_SECS` (default 30): server-side limit per statement
pub fn database_from_env() -> Result<DbSettings, String> {
    let url = opt("DATABASE_URL").ok_or(
        "DATABASE_URL must be set, e.g. postgres://hourwise:password@localhost:5432/hourwise",
    )?;
    let mut settings = DbSettings::new(&url)?;
    if let Some(n) = opt_num::<u32>("DATABASE_MAX_CONNECTIONS").filter(|n| *n > 0) {
        settings.max_connections = n;
    }
    if let Some(s) = opt_num::<u64>("DATABASE_ACQUIRE_TIMEOUT_SECS").filter(|s| *s > 0) {
        settings.acquire_timeout = Duration::from_secs(s);
    }
    if let Some(s) = opt_num::<u64>("DATABASE_STATEMENT_TIMEOUT_SECS").filter(|s| *s > 0) {
        settings.statement_timeout = Duration::from_secs(s);
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_secrets() {
        let config = Config {
            database: DbSettings::new("postgres://app:db-pass-123@db:5432/hourwise")
                .unwrap_or_else(|e| panic!("{e}")),
            bind_addr: "0.0.0.0:8080".into(),
            jwt_secret: "jwt-secret-value-that-is-long-enough-123".into(),
            jwt_ttl_hours: 1,
            upload_dir: "/data/uploads".into(),
            cors_origin: None,
            admin_email: Some("admin@example.com".into()),
            admin_password: Some("admin-pass-456".into()),
            admin_name: "Admin".into(),
            cookie_secure: true,
            login_max_attempts: 5,
            login_max_attempts_per_ip: 20,
            login_lockout_secs: 900,
            trusted_proxies: Vec::new(),
            password_hash_concurrency: 4,
        };
        let debug = format!("{config:?}");
        for secret in ["db-pass-123", "jwt-secret-value", "admin-pass-456"] {
            assert!(!debug.contains(secret), "{secret} leaked: {debug}");
        }
        assert!(debug.contains("app@db:5432/hourwise"));
    }
}
