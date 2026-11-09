//! Hourwise API: library crate so the binary, the CLI commands and the integration tests all
//! build the exact same application.

pub mod auth;
pub mod backup;
pub mod config;
pub mod db;
pub mod error;
pub mod migrate;
pub mod models;
pub mod routes;

use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    http::{header::CONTENT_TYPE, HeaderName, HeaderValue, Method},
    Router,
};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

use crate::{config::Config, db::Db, error::AppResult};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub login_limiter: Arc<auth::LoginLimiter>,
}

/// Connects to the database, applies migrations, prepares the upload directory and seeds the
/// first admin. Used by `serve` and by the integration tests.
pub async fn init_state(config: Config, connect_attempts: u32) -> AppResult<AppState> {
    tracing::info!("using {}", config.database.describe());
    let db = db::open_with_retry(&config.database, connect_attempts).await?;
    migrate::run(&db).await?;
    std::fs::create_dir_all(format!("{}/avatars", config.upload_dir)).map_err(|e| {
        error::AppError::Internal(format!("failed to create upload directory: {e}"))
    })?;
    seed_admin(&db, &config).await?;
    let login_limiter = Arc::new(auth::LoginLimiter::new(
        config.login_max_attempts,
        std::time::Duration::from_secs(config.login_lockout_secs),
    ));
    Ok(AppState {
        db,
        config: Arc::new(config),
        login_limiter,
    })
}

/// Builds the full HTTP application (API + uploaded files).
pub fn app(state: AppState) -> Router {
    let config = state.config.clone();
    let mut app = Router::new()
        .nest("/api", routes::router())
        .nest_service("/uploads", ServeDir::new(&config.upload_dir))
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024))
        .layer(TraceLayer::new_for_http());

    if let Some(origin) = &config.cors_origin {
        match origin.parse::<HeaderValue>() {
            Ok(origin) => {
                app = app.layer(
                    CorsLayer::new()
                        .allow_origin(origin)
                        .allow_methods([
                            Method::GET,
                            Method::POST,
                            Method::PUT,
                            Method::PATCH,
                            Method::DELETE,
                        ])
                        .allow_headers([CONTENT_TYPE, HeaderName::from_static(auth::CSRF_HEADER)])
                        .allow_credentials(true),
                );
            }
            Err(_) => tracing::error!("CORS_ORIGIN is not a valid origin; CORS disabled"),
        }
    }
    app.with_state(state)
}

/// Creates the first admin account from ADMIN_EMAIL / ADMIN_PASSWORD when the
/// database has no admin yet. Safe to run on every start.
pub async fn seed_admin(db: &Db, config: &Config) -> AppResult<()> {
    let (Some(email), Some(password)) = (&config.admin_email, &config.admin_password) else {
        return Ok(());
    };
    let conn = db.conn().await?;
    let admins = db::query_i64(
        &conn,
        "SELECT COUNT(*) FROM users WHERE role = 'admin'",
        args![],
    )
    .await?;
    if admins > 0 {
        return Ok(());
    }
    let hash = auth::hash_password(password.clone()).await?;
    let now = db::now_ts();
    db::execute(
        &conn,
        "INSERT INTO users (id, email, name, password_hash, role, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'admin', ?5, ?5)
         ON CONFLICT (email) DO UPDATE SET role = 'admin', is_active = 1, updated_at = ?5",
        args![Uuid::new_v4(), email, &config.admin_name, hash, now],
    )
    .await?;
    tracing::info!("Created initial admin account {email}");
    Ok(())
}
