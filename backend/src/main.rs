mod auth;
mod config;
mod error;
mod models;
mod routes;

use std::{sync::Arc, time::Duration};

use axum::{
    extract::DefaultBodyLimit,
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE},
        HeaderValue, Method,
    },
    Router,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("hourwise_backend=info,tower_http=info")),
        )
        .init();

    let config = Config::from_env();

    let db = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await
        .expect("failed to connect to the database");

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("failed to run database migrations");

    std::fs::create_dir_all(format!("{}/avatars", config.upload_dir))
        .expect("failed to create upload directory");

    seed_admin(&db, &config).await;

    let state = AppState {
        db,
        config: Arc::new(config.clone()),
    };

    let mut app = Router::new()
        .nest("/api", routes::router())
        .nest_service("/uploads", ServeDir::new(&config.upload_dir))
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024))
        .layer(TraceLayer::new_for_http());

    if let Some(origin) = &config.cors_origin {
        let origin: HeaderValue = origin.parse().expect("CORS_ORIGIN is not a valid origin");
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
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        );
    }

    let app = app.with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .expect("failed to bind address");
    tracing::info!("Hourwise API listening on {}", config.bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}

/// Creates the first admin account from ADMIN_EMAIL / ADMIN_PASSWORD when the
/// database has no admin yet. Safe to run on every start.
async fn seed_admin(db: &PgPool, config: &Config) {
    let (Some(email), Some(password)) = (&config.admin_email, &config.admin_password) else {
        return;
    };
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(db)
        .await
        .expect("failed to count admins");
    if admins > 0 {
        return;
    }
    let hash = auth::hash_password(password.clone())
        .await
        .expect("failed to hash admin password");
    sqlx::query(
        "INSERT INTO users (id, email, name, password_hash, role) VALUES ($1, $2, $3, $4, 'admin')
         ON CONFLICT (email) DO UPDATE SET role = 'admin', is_active = TRUE",
    )
    .bind(Uuid::new_v4())
    .bind(email)
    .bind(&config.admin_name)
    .bind(hash)
    .execute(db)
    .await
    .expect("failed to create the initial admin");
    tracing::info!("Created initial admin account {email}");
}
