use std::env;

use crate::db::DbTarget;

#[derive(Debug, Clone)]
pub struct Config {
    pub database: DbTarget,
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
    /// Failed logins allowed per email before it is locked for `login_lockout_secs`.
    pub login_max_attempts: u32,
    pub login_lockout_secs: u64,
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
            login_lockout_secs: opt("LOGIN_LOCKOUT_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(15 * 60),
        }
    }
}

/// Database settings only (used by the `migrate` / `verify` / `backup` commands, which
/// don't need the JWT secret).
pub fn database_from_env() -> Result<DbTarget, String> {
    let Some(url) = opt("TURSO_DATABASE_URL") else {
        return Err(
            "TURSO_DATABASE_URL must be set (libsql://<db>-<org>.turso.io for Turso \
                    Cloud, or file:./data/hourwise.db for a local database file)"
                .into(),
        );
    };
    DbTarget::parse(&url, opt("TURSO_AUTH_TOKEN"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_database_targets() {
        assert_eq!(
            DbTarget::parse("file:./data/x.db", None),
            Ok(DbTarget::Local {
                path: "./data/x.db".into()
            })
        );
        assert_eq!(
            DbTarget::parse("/var/lib/hw.db", None),
            Ok(DbTarget::Local {
                path: "/var/lib/hw.db".into()
            })
        );
        assert!(DbTarget::parse("libsql://hw-org.turso.io", None).is_err());
        assert!(DbTarget::parse("libsql://hw-org.turso.io", Some(String::new())).is_err());
        let remote = DbTarget::parse("libsql://hw-org.turso.io", Some("tok".into()));
        assert!(remote.as_ref().is_ok_and(DbTarget::is_remote));
        // The token must never appear in log output.
        assert!(!remote
            .map(|r| r.describe())
            .unwrap_or_default()
            .contains("tok"));
        // A local sqld / `turso dev` server needs no token.
        assert!(DbTarget::parse("http://127.0.0.1:8081", None).is_ok());
        assert!(DbTarget::parse(":memory:", None).is_err());
        assert!(DbTarget::parse("file:", None).is_err());
    }
}
