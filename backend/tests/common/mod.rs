//! Shared helpers for the integration tests.
//!
//! Every [`TestApp`] gets its own **freshly created** PostgreSQL database (named
//! `hourwise_test_<random>`) on a disposable test server, plus its own upload folder, so tests are
//! isolated from each other. Requests go through the full axum router (extractors, CSRF,
//! cookies) without a network socket.
//!
//! Safety rules, checked before anything is created or written:
//! - The server comes only from `HOURWISE_TEST_DATABASE_URL`; there is no fallback to
//!   `DATABASE_URL`.
//! - It is refused if it is the same server (host + port, ignoring formatting and credentials) as
//!   `DATABASE_URL` or any entry of `HOURWISE_PROTECTED_DATABASES` (forwarded by
//!   `scripts/cargo.mjs` from your `.env` files, without credentials).
//! - The server must identify itself as disposable: `docker-compose.test.yml` starts PostgreSQL
//!   with `-c hourwise.disposable=on`. Production servers don't have that setting.
//! - A new database must be empty before migrations run.
//! - Only databases this process created are dropped, and only if their name has the test prefix.
//! - Passwords are never printed.
#![allow(dead_code, clippy::unwrap_used)]

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{header, HeaderMap, Method, Request, StatusCode},
    Router,
};
use chrono::{NaiveDate, Utc};
use hourwise_backend::{
    args,
    auth::{HashLimiter, LoginLimiter},
    config::Config,
    db::{self, Db, DbSettings, Destination},
    AppState,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::{ConnectOptions, Connection};
use tempfile::TempDir;
use tower::ServiceExt;

pub const ADMIN_EMAIL: &str = "admin@test.local";
pub const ADMIN_PASSWORD: &str = "AdminPass123";
pub const JWT_SECRET: &str = "test-secret-test-secret-test-secret-0123";
pub const TEST_DB_PREFIX: &str = "hourwise_test_";

/// A signed-in browser: the cookie header to send and the CSRF token to echo.
#[derive(Clone, Debug)]
pub struct Session {
    pub cookie: String,
    pub csrf: String,
}

pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Response {
    pub fn id(&self) -> String {
        self.body["id"].as_str().unwrap_or_default().to_string()
    }
    pub fn error(&self) -> String {
        self.body["error"].as_str().unwrap_or_default().to_string()
    }
}

// ---------------------------------------------------------------- disposable databases

/// The disposable test server, after the production-destination checks.
pub fn test_server() -> DbSettings {
    let url = std::env::var("HOURWISE_TEST_DATABASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .expect(
            "HOURWISE_TEST_DATABASE_URL is not set. Run the tests with `npm run test:backend`, \
             which starts the disposable server from docker-compose.test.yml",
        );
    let settings =
        DbSettings::new(&url).unwrap_or_else(|e| panic!("HOURWISE_TEST_DATABASE_URL: {e}"));
    let server = settings.destination().unwrap();
    if let Err(e) = check_not_protected(&server, &protected_destinations()) {
        panic!("{e}");
    }
    settings
}

/// Refuses a test server that is (or shares a server with) a protected database.
pub fn check_not_protected(server: &Destination, protected: &[Destination]) -> Result<(), String> {
    match protected.iter().find(|p| server.same_server(p)) {
        Some(p) => Err(format!(
            "refusing to run: HOURWISE_TEST_DATABASE_URL ({server}) is on the same server as a \
             protected database ({p}). Use the disposable test server instead."
        )),
        None => Ok(()),
    }
}

/// `DATABASE_URL` plus the comma-separated `host:port/db` entries in
/// `HOURWISE_PROTECTED_DATABASES`.
pub fn protected_destinations() -> Vec<Destination> {
    let mut out = Vec::new();
    if let Ok(url) = std::env::var("DATABASE_URL") {
        if let Ok(d) = db::destination(&url) {
            out.push(d);
        }
    }
    if let Ok(list) = std::env::var("HOURWISE_PROTECTED_DATABASES") {
        for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            match db::destination(&format!("postgres://protected@{entry}")) {
                Ok(d) => out.push(d),
                Err(e) => panic!("HOURWISE_PROTECTED_DATABASES entry {entry:?}: {e}"),
            }
        }
    }
    out
}

/// A database created for one test (shared by [`TestApp::restart`]). It is dropped when the
/// last reference goes away.
pub struct TestDatabase {
    pub name: String,
    pub settings: DbSettings,
    server: DbSettings,
}

async fn admin_connection(server: &DbSettings) -> sqlx::PgConnection {
    server
        .connect_options()
        .unwrap()
        .connect()
        .await
        .unwrap_or_else(|e| {
            panic!(
                "cannot connect to the test server {}: {e}",
                server.describe()
            )
        })
}

impl TestDatabase {
    pub async fn create() -> Arc<Self> {
        let server = test_server();
        let mut admin = admin_connection(&server).await;

        // The server must say it is disposable before we create or write anything.
        let marker: Option<String> =
            sqlx::query_scalar("SELECT current_setting('hourwise.disposable', true)")
                .fetch_one(&mut admin)
                .await
                .unwrap();
        assert_eq!(
            marker.as_deref(),
            Some("on"),
            "refusing to run: {} is not marked as a disposable test server \
             (start it with docker-compose.test.yml, which sets hourwise.disposable=on)",
            server.describe()
        );

        let name = format!("{TEST_DB_PREFIX}{}", uuid::Uuid::new_v4().simple());
        assert!(name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
        // The name is generated above (never input), so it is safe as an identifier.
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&mut admin)
            .await
            .unwrap();
        admin.close().await.ok();

        let mut settings = server.with_database(&name);
        settings.max_connections = 8;
        settings.acquire_timeout = Duration::from_secs(10);

        // Check the destination before migrating: it must be the brand-new, empty database.
        let db = Db::connect(&settings).await.unwrap();
        let mut conn = db.conn().await.unwrap();
        let tables = db::query_i64(
            &mut conn,
            "SELECT COUNT(*) FROM pg_tables WHERE schemaname = 'public'",
            args![],
        )
        .await
        .unwrap();
        assert_eq!(tables, 0, "refusing to continue: {name} is not empty");
        drop(conn);
        db.close().await;

        Arc::new(Self {
            name,
            settings,
            server,
        })
    }

    /// A pool on this database (without running the app's startup).
    pub async fn connect(&self) -> Db {
        Db::connect(&self.settings).await.unwrap()
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        // Only ever drop a database this process created, and only with the test prefix.
        if !self.name.starts_with(TEST_DB_PREFIX) {
            return;
        }
        let name = self.name.clone();
        let server = self.server.clone();
        // Drop runs inside the test's runtime; do the async work on a separate thread + runtime.
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            rt.block_on(async move {
                let mut admin = server.connect_options().ok()?.connect().await.ok()?;
                // FORCE closes the test app's remaining pooled connections (PostgreSQL 13+).
                sqlx::query(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
                    .execute(&mut admin)
                    .await
                    .ok()?;
                admin.close().await.ok()
            })
        })
        .join();
    }
}

// ---------------------------------------------------------------- the app

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub dir: Arc<TempDir>,
    pub database: Arc<TestDatabase>,
}

pub fn test_config(dir: &TempDir, database: &TestDatabase) -> Config {
    Config {
        database: database.settings.clone(),
        bind_addr: "127.0.0.1:0".into(),
        jwt_secret: JWT_SECRET.into(),
        jwt_ttl_hours: 1,
        upload_dir: dir.path().join("uploads").to_string_lossy().into_owned(),
        cors_origin: None,
        admin_email: Some(ADMIN_EMAIL.into()),
        admin_password: Some(ADMIN_PASSWORD.into()),
        admin_name: "Test Admin".into(),
        cookie_secure: true,
        login_max_attempts: 50,
        login_max_attempts_per_ip: 1000,
        login_lockout_secs: 60,
        trusted_proxies: Vec::new(),
        password_hash_concurrency: 4,
    }
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with(|_| {}).await
    }

    /// Starts an app with a customised config (e.g. a lower login limit).
    pub async fn with(customise: impl FnOnce(&mut Config)) -> Self {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let database = TestDatabase::create().await;
        let mut config = test_config(&dir, &database);
        customise(&mut config);
        Self::start(config, dir, database).await
    }

    pub async fn start(config: Config, dir: Arc<TempDir>, database: Arc<TestDatabase>) -> Self {
        let state = hourwise_backend::init_state(config, 1).await.unwrap();
        let router = hourwise_backend::app(state.clone());
        TestApp {
            router,
            state,
            dir,
            database,
        }
    }

    /// Restarts the application (new pool, new state) on the same database and upload directory.
    pub async fn restart(&self) -> Self {
        let config = (*self.state.config).clone();
        Self::start(config, self.dir.clone(), self.database.clone()).await
    }

    /// An app whose state uses a different (e.g. unreachable) database, sharing everything else.
    pub fn with_db(&self, db: Db) -> Router {
        let state = AppState {
            db,
            config: self.state.config.clone(),
            login_limiter: Arc::new(LoginLimiter::new(5, 20, Duration::from_secs(60))),
            hashing: Arc::new(HashLimiter::new(4, Duration::from_secs(5))),
        };
        hourwise_backend::app(state)
    }

    /// The same app (same database and config) with a different password-hashing limit.
    pub fn with_hashing(&self, hashing: HashLimiter) -> Router {
        let state = AppState {
            hashing: Arc::new(hashing),
            ..self.state.clone()
        };
        hourwise_backend::app(state)
    }

    /// A login that arrives over TCP from `peer`, optionally with an `X-Forwarded-For` header.
    pub async fn try_login_from(
        &self,
        email: &str,
        password: &str,
        peer: &str,
        forwarded_for: Option<&str>,
    ) -> Response {
        send_to(
            &self.router,
            login_request(email, password, peer, forwarded_for),
        )
        .await
    }

    /// A connection to this test's database.
    pub async fn conn(&self) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
        self.state.db.conn().await.unwrap()
    }

    pub async fn send(&self, req: Request<Body>) -> Response {
        send_to(&self.router, req).await
    }

    /// JSON request. Adds the session cookie and, for non-GET requests, the CSRF header.
    pub async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        session: Option<&Session>,
    ) -> Response {
        self.send(build(method, path, body, session, true)).await
    }

    pub async fn get(&self, path: &str, s: &Session) -> Response {
        self.call(Method::GET, path, None, Some(s)).await
    }
    pub async fn post(&self, path: &str, body: Value, s: &Session) -> Response {
        self.call(Method::POST, path, Some(body), Some(s)).await
    }
    pub async fn patch(&self, path: &str, body: Value, s: &Session) -> Response {
        self.call(Method::PATCH, path, Some(body), Some(s)).await
    }
    pub async fn put(&self, path: &str, body: Value, s: &Session) -> Response {
        self.call(Method::PUT, path, Some(body), Some(s)).await
    }
    pub async fn delete(&self, path: &str, s: &Session) -> Response {
        self.call(Method::DELETE, path, None, Some(s)).await
    }

    pub async fn try_login(&self, email: &str, password: &str) -> Response {
        self.call(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": password })),
            None,
        )
        .await
    }

    pub async fn login(&self, email: &str, password: &str) -> Session {
        let res = self.try_login(email, password).await;
        assert_eq!(res.status, StatusCode::OK, "login failed: {}", res.body);
        session_from(&res.headers).expect("login did not set session cookies")
    }

    pub async fn admin(&self) -> Session {
        self.login(ADMIN_EMAIL, ADMIN_PASSWORD).await
    }

    /// Creates a user through the admin API and returns its id.
    pub async fn create_user(
        &self,
        admin: &Session,
        email: &str,
        password: &str,
        role: &str,
    ) -> String {
        let res = self
            .post(
                "/api/admin/users",
                json!({ "email": email, "name": email.split('@').next(), "password": password, "role": role }),
                admin,
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        res.id()
    }

    pub async fn create_project(&self, admin: &Session, name: &str) -> String {
        let res = self
            .post("/api/admin/projects", json!({ "name": name }), admin)
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        res.id()
    }

    pub async fn add_member(&self, admin: &Session, project: &str, user: &str) {
        let res = self
            .post(
                &format!("/api/admin/projects/{project}/members"),
                json!({ "user_ids": [user] }),
                admin,
            )
            .await;
        assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    }

    /// A member with a password, signed in, who belongs to a fresh project.
    pub async fn member_on_project(
        &self,
        admin: &Session,
        email: &str,
    ) -> (String, String, Session) {
        let user = self
            .create_user(admin, email, "memberpass1", "member")
            .await;
        let project = self
            .create_project(admin, &format!("Project for {email}"))
            .await;
        self.add_member(admin, &project, &user).await;
        let session = self.login(email, "memberpass1").await;
        (user, project, session)
    }

    pub async fn log_time(
        &self,
        s: &Session,
        project: &str,
        date: NaiveDate,
        minutes: i64,
        description: &str,
    ) -> Response {
        self.post(
            "/api/time-entries",
            json!({
                "project_id": project,
                "entry_date": date.to_string(),
                "minutes": minutes,
                "description": description
            }),
            s,
        )
        .await
    }
}

pub fn build(
    method: Method,
    path: &str,
    body: Option<Value>,
    session: Option<&Session>,
    with_csrf: bool,
) -> Request<Body> {
    let mut req = Request::builder().method(method.clone()).uri(path);
    if let Some(s) = session {
        req = req.header(header::COOKIE, &s.cookie);
        if with_csrf && method != Method::GET {
            req = req.header("x-csrf-token", &s.csrf);
        }
    }
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    req.body(body).unwrap()
}

/// `POST /api/auth/login` as if it arrived over TCP from `peer` (`ConnectInfo`, which `main.rs`
/// installs in production).
pub fn login_request(
    email: &str,
    password: &str,
    peer: &str,
    forwarded_for: Option<&str>,
) -> Request<Body> {
    let mut req = build(
        Method::POST,
        "/api/auth/login",
        Some(json!({ "email": email, "password": password })),
        None,
        true,
    );
    let peer: std::net::SocketAddr = format!("{peer}:40000").parse().unwrap();
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(peer));
    if let Some(xff) = forwarded_for {
        req.headers_mut()
            .insert("x-forwarded-for", xff.parse().unwrap());
    }
    req
}

pub async fn send_to(router: &Router, req: Request<Body>) -> Response {
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Response {
        status,
        headers,
        body,
    }
}

/// Extracts the session from `Set-Cookie` headers.
pub fn session_from(headers: &HeaderMap) -> Option<Session> {
    let mut session = None;
    let mut csrf = None;
    for v in headers.get_all(header::SET_COOKIE) {
        let v = v.to_str().ok()?;
        let pair = v.split(';').next()?;
        let (k, val) = pair.split_once('=')?;
        match k {
            "hw_session" if !val.is_empty() => session = Some(val.to_string()),
            "hw_csrf" if !val.is_empty() => csrf = Some(val.to_string()),
            _ => {}
        }
    }
    let (session, csrf) = (session?, csrf?);
    Some(Session {
        cookie: format!("hw_session={session}; hw_csrf={csrf}"),
        csrf,
    })
}

pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

pub fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// Sum of `minutes` over a JSON array of entries.
pub fn total_minutes(entries: &Value) -> i64 {
    entries
        .as_array()
        .map(|a| a.iter().map(|e| e["minutes"].as_i64().unwrap_or(0)).sum())
        .unwrap_or(0)
}
