use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub jwt_ttl_hours: i64,
    pub upload_dir: String,
    pub cors_origin: Option<String>,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
    pub admin_name: String,
}

fn opt(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Self {
        let jwt_secret = opt("JWT_SECRET").expect("JWT_SECRET must be set");
        if jwt_secret.len() < 32 {
            panic!("JWT_SECRET must be at least 32 characters long");
        }
        Self {
            database_url: opt("DATABASE_URL").expect("DATABASE_URL must be set"),
            bind_addr: opt("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()),
            jwt_secret,
            jwt_ttl_hours: opt("JWT_TTL_HOURS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(24 * 7),
            upload_dir: opt("UPLOAD_DIR").unwrap_or_else(|| "./uploads".into()),
            cors_origin: opt("CORS_ORIGIN"),
            admin_email: opt("ADMIN_EMAIL").map(|e| e.to_lowercase()),
            admin_password: opt("ADMIN_PASSWORD"),
            admin_name: opt("ADMIN_NAME").unwrap_or_else(|| "Administrator".into()),
        }
    }
}
