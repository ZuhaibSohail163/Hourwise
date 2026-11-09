//! Shared helpers for the integration tests.
//!
//! Every [`TestApp`] gets its own temporary directory holding a fresh libSQL database file and
//! upload folder, so tests are isolated from each other and never touch a real database.
//! Requests go through the full axum router (extractors, CSRF, cookies) without a network socket.
#![allow(dead_code, clippy::unwrap_used)]

use std::{sync::Arc, time::Duration};

use axum::{
    body::Body,
    http::{header, HeaderMap, Method, Request, StatusCode},
    Router,
};
use chrono::{NaiveDate, Utc};
use hourwise_backend::{auth::LoginLimiter, config::Config, db::DbTarget, AppState};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

pub const ADMIN_EMAIL: &str = "admin@test.local";
pub const ADMIN_PASSWORD: &str = "AdminPass123";
pub const JWT_SECRET: &str = "test-secret-test-secret-test-secret-0123";

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

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub dir: Arc<TempDir>,
}

pub fn test_config(dir: &TempDir) -> Config {
    Config {
        database: DbTarget::Local {
            path: dir
                .path()
                .join("hourwise.db")
                .to_string_lossy()
                .into_owned(),
        },
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
        login_lockout_secs: 60,
    }
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with(|_| {}).await
    }

    /// Starts an app with a customised config (e.g. a lower login limit).
    pub async fn with(customise: impl FnOnce(&mut Config)) -> Self {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let mut config = test_config(&dir);
        customise(&mut config);
        Self::start(config, dir).await
    }

    /// Starts an app against an explicit database target (used by the Turso Cloud tests).
    pub async fn with_target(target: DbTarget) -> Self {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let mut config = test_config(&dir);
        config.database = target;
        Self::start(config, dir).await
    }

    pub async fn start(config: Config, dir: Arc<TempDir>) -> Self {
        let state = hourwise_backend::init_state(config, 1).await.unwrap();
        let router = hourwise_backend::app(state.clone());
        TestApp { router, state, dir }
    }

    /// Restarts the application on the same database and upload directory.
    pub async fn restart(&self) -> Self {
        let config = (*self.state.config).clone();
        Self::start(config, self.dir.clone()).await
    }

    /// An app whose state uses a different (e.g. unreachable) database, sharing everything else.
    pub fn with_db(&self, db: hourwise_backend::db::Db) -> Router {
        let state = AppState {
            db,
            config: self.state.config.clone(),
            login_limiter: Arc::new(LoginLimiter::new(5, Duration::from_secs(60))),
        };
        hourwise_backend::app(state)
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
