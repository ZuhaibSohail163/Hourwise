mod admin;
mod auth;
mod entries;
mod me;
mod projects;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, patch, post, put},
    Json, Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde_json::{json, Value};

use crate::{error::AppError, AppState};

/// Liveness: the process is up and serving HTTP. Doesn't touch the database, so a database
/// outage doesn't make the orchestrator restart healthy backend containers.
async fn health() -> &'static str {
    "ok"
}

/// Readiness: the database answers within 3 seconds. Use it for load balancers / monitoring.
/// The error detail is logged, never returned.
async fn ready(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match tokio::time::timeout(std::time::Duration::from_secs(3), state.db.ping()).await {
        Ok(Ok(())) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "database": "ok" })),
        ),
        Ok(Err(e)) => {
            tracing::warn!("readiness check failed: {}", e.detail());
            unavailable()
        }
        Err(_) => {
            tracing::warn!("readiness check timed out");
            unavailable()
        }
    }
}

fn unavailable() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "status": "unavailable", "database": "unavailable" })),
    )
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/health/ready", get(ready))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/me", get(me::get_me).patch(me::update_me))
        .route("/me/password", put(me::change_password))
        .route(
            "/me/avatar",
            post(me::upload_avatar).delete(me::delete_avatar),
        )
        .route("/projects", get(projects::my_projects))
        .route("/time-entries", get(entries::list).post(entries::create))
        .route(
            "/time-entries/{id}",
            patch(entries::update).delete(entries::remove),
        )
        .nest("/admin", admin::router())
}

/// Resolves an optional `from`/`to` query range. Defaults to the current month.
pub(crate) fn date_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<(NaiveDate, NaiveDate), AppError> {
    let today = Utc::now().date_naive();
    let from = from.unwrap_or_else(|| today.with_day(1).unwrap_or(today));
    let to = to.unwrap_or(today);
    if to < from {
        return Err(AppError::bad("`to` must not be before `from`"));
    }
    if (to - from) > Duration::days(400) {
        return Err(AppError::bad("Date range cannot exceed 400 days"));
    }
    Ok((from, to))
}

/// Trims a string and turns empty strings into `None`.
pub(crate) fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub(crate) fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.contains(char::is_whitespace)
}

pub(crate) fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_default()
    }

    #[test]
    fn date_range_validates_bounds() {
        let (from, to) =
            date_range(Some(day("2026-01-01")), Some(day("2026-01-31"))).unwrap_or_default();
        assert_eq!((from, to), (day("2026-01-01"), day("2026-01-31")));
        // A single day is a valid range.
        assert!(date_range(Some(day("2026-02-28")), Some(day("2026-02-28"))).is_ok());
        assert!(date_range(Some(day("2026-02-02")), Some(day("2026-02-01"))).is_err());
        // 400 days is the maximum span.
        assert!(date_range(Some(day("2025-01-01")), Some(day("2026-02-05"))).is_ok());
        assert!(date_range(Some(day("2025-01-01")), Some(day("2026-02-06"))).is_err());
    }

    #[test]
    fn date_range_defaults_to_current_month() {
        let today = Utc::now().date_naive();
        let (from, to) = date_range(None, None).unwrap_or_default();
        assert_eq!(from.day(), 1);
        assert_eq!((from.month(), from.year()), (today.month(), today.year()));
        assert_eq!(to, today);
    }

    #[test]
    fn clean_trims_and_drops_empty() {
        assert_eq!(clean(Some("  x ".into())), Some("x".into()));
        assert_eq!(clean(Some("   ".into())), None);
        assert_eq!(clean(None), None);
    }

    #[test]
    fn email_and_color_validators() {
        assert!(valid_email("a@b.co"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("@b.co"));
        assert!(!valid_email("a b@c.co"));
        assert!(!valid_email("a@.co"));
        assert!(!valid_email("a@co."));
        assert!(valid_color("#a1B2c3"));
        assert!(!valid_color("a1b2c3"));
        assert!(!valid_color("#a1b2c"));
        assert!(!valid_color("#a1b2cz"));
    }
}
