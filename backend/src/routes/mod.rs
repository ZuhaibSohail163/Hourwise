mod admin;
mod auth;
mod entries;
mod me;
mod projects;

use axum::{
    routing::{get, patch, post, put},
    Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};

use crate::{error::AppError, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/auth/login", post(auth::login))
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
