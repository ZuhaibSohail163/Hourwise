use axum::{
    http::{header::RETRY_AFTER, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("Invalid credentials or session expired")]
    Unauthorized,
    #[error("You do not have permission to do that")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Too many failed attempts. Try again in {0} seconds.")]
    TooManyRequests(u64),
    #[error("The database is temporarily unavailable. Please try again.")]
    Unavailable(String),
    #[error("Internal server error")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }
    pub fn not_found(what: &str) -> Self {
        Self::NotFound(format!("{what} not found"))
    }
    /// Full detail for logs (never sent to clients for 5xx errors).
    pub fn detail(&self) -> String {
        match self {
            AppError::Internal(d) | AppError::Unavailable(d) => d.clone(),
            other => other.to_string(),
        }
    }
}

/// Maps a database error message to an [`AppError`]. SQLite reports constraint failures with
/// the same wording locally and through Turso Cloud, so matching on the message works for both.
pub fn classify_db_error(message: &str) -> AppError {
    let m = message.to_ascii_lowercase();
    if m.contains("unique constraint failed") || m.contains("sqlite_constraint_primarykey") {
        return AppError::Conflict("A record with that value already exists".into());
    }
    if m.contains("foreign key constraint failed") {
        return AppError::BadRequest("Referenced record does not exist".into());
    }
    if m.contains("check constraint failed") || m.contains("not null constraint failed") {
        return AppError::BadRequest("Value is out of the allowed range".into());
    }
    if m.contains("database is locked") || m.contains("sqlite_busy") {
        return AppError::Unavailable(message.to_string());
    }
    AppError::Internal(message.to_string())
}

impl From<libsql::Error> for AppError {
    fn from(e: libsql::Error) -> Self {
        match &e {
            libsql::Error::ConnectionFailed(_) => AppError::Unavailable(e.to_string()),
            libsql::Error::QueryReturnedNoRows => AppError::NotFound("Not found".into()),
            libsql::Error::Hrana(inner) => {
                let msg = inner.to_string();
                let lower = msg.to_ascii_lowercase();
                // Network / auth failures talking to Turso Cloud become 503s; SQL errors
                // (constraint violations etc.) are classified like local SQLite errors.
                let is_sql_error = lower.contains("constraint") || lower.contains("sqlite");
                let is_transport_error = [
                    "error trying to connect",
                    "connection refused",
                    "timed out",
                    "dns",
                    "unauthorized",
                    "status=401",
                    "status=403",
                    "status=5",
                    "stream expired",
                ]
                .iter()
                .any(|needle| lower.contains(needle));
                if is_transport_error && !is_sql_error {
                    return AppError::Unavailable(msg);
                }
                classify_db_error(&msg)
            }
            _ => classify_db_error(&e.to_string()),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            AppError::Unavailable(detail) => {
                tracing::error!("database unavailable: {detail}");
                StatusCode::SERVICE_UNAVAILABLE
            }
            AppError::Internal(detail) => {
                tracing::error!("internal error: {detail}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let mut res = (status, Json(json!({ "error": self.to_string() }))).into_response();
        if let AppError::TooManyRequests(secs) = &self {
            if let Ok(v) = secs.to_string().parse() {
                res.headers_mut().insert(RETRY_AFTER, v);
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_constraint_messages() {
        assert!(matches!(
            classify_db_error("SQLite failure: `UNIQUE constraint failed: users.email`"),
            AppError::Conflict(_)
        ));
        assert!(matches!(
            classify_db_error("FOREIGN KEY constraint failed"),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            classify_db_error("CHECK constraint failed: minutes > 0"),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            classify_db_error("database is locked"),
            AppError::Unavailable(_)
        ));
        assert!(matches!(
            classify_db_error("no such table: foo"),
            AppError::Internal(_)
        ));
    }

    #[test]
    fn status_codes() {
        assert_eq!(
            AppError::Unauthorized.into_response().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            AppError::Forbidden.into_response().status(),
            StatusCode::FORBIDDEN
        );
        let r = AppError::TooManyRequests(30).into_response();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            r.headers().get(RETRY_AFTER).map(|v| v.to_str().ok()),
            Some(Some("30"))
        );
        assert_eq!(
            AppError::Unavailable("x".into()).into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
