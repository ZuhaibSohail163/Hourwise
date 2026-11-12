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
    /// Too many password checks are already running (see `auth::HashLimiter`).
    #[error("The server is busy. Please try again in a moment.")]
    Busy,
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

/// Maps a PostgreSQL SQLSTATE code (see the PostgreSQL "Error Codes" appendix) to an
/// [`AppError`]. `detail` is only used for logs.
pub fn classify_sql_state(code: &str, detail: &str) -> AppError {
    match code {
        // unique_violation
        "23505" => AppError::Conflict("A record with that value already exists".into()),
        // foreign_key_violation
        "23503" => AppError::BadRequest("Referenced record does not exist".into()),
        // check_violation, not_null_violation, numeric_value_out_of_range,
        // string_data_right_truncation, invalid_datetime_format, datetime_field_overflow
        "23514" | "23502" | "22003" | "22001" | "22007" | "22008" => {
            AppError::BadRequest("Value is out of the allowed range".into())
        }
        // serialization_failure, deadlock_detected, lock_not_available, query_canceled
        // (statement_timeout), too_many_connections, cannot_connect_now, admin_shutdown,
        // crash_shutdown
        "40001" | "40P01" | "55P03" | "57014" | "53300" | "57P03" | "57P01" | "57P02" => {
            AppError::Unavailable(format!("{code}: {detail}"))
        }
        // Class 08: connection exceptions
        c if c.starts_with("08") => AppError::Unavailable(format!("{code}: {detail}")),
        _ => AppError::Internal(format!("{code}: {detail}")),
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::RowNotFound => AppError::NotFound("Not found".into()),
            sqlx::Error::Database(db) => {
                classify_sql_state(db.code().as_deref().unwrap_or(""), db.message())
            }
            // Pool exhausted, PostgreSQL down or restarting, network or TLS failures.
            sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::Protocol(_)
            | sqlx::Error::WorkerCrashed => AppError::Unavailable(e.to_string()),
            _ => AppError::Internal(e.to_string()),
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
            AppError::Busy => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Internal(detail) => {
                tracing::error!("internal error: {detail}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let mut res = (status, Json(json!({ "error": self.to_string() }))).into_response();
        let retry_after = match &self {
            AppError::TooManyRequests(secs) => Some(*secs),
            AppError::Busy => Some(2),
            _ => None,
        };
        if let Some(Ok(v)) = retry_after.map(|s| s.to_string().parse()) {
            res.headers_mut().insert(RETRY_AFTER, v);
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_postgres_error_codes() {
        assert!(matches!(
            classify_sql_state("23505", "duplicate key value violates unique constraint"),
            AppError::Conflict(_)
        ));
        assert!(matches!(
            classify_sql_state("23503", "violates foreign key constraint"),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            classify_sql_state("23514", "violates check constraint"),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            classify_sql_state("23502", "null value in column"),
            AppError::BadRequest(_)
        ));
        assert!(matches!(
            classify_sql_state("57014", "canceling statement due to statement timeout"),
            AppError::Unavailable(_)
        ));
        assert!(matches!(
            classify_sql_state("08006", "connection failure"),
            AppError::Unavailable(_)
        ));
        assert!(matches!(
            classify_sql_state("42P01", "relation does not exist"),
            AppError::Internal(_)
        ));
        assert!(matches!(
            AppError::from(sqlx::Error::PoolTimedOut),
            AppError::Unavailable(_)
        ));
        assert!(matches!(
            AppError::from(sqlx::Error::RowNotFound),
            AppError::NotFound(_)
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
