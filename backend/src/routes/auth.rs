use axum::{
    extract::State,
    http::{header::SET_COOKIE, HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    args,
    auth::{clear_cookies, session_cookies, verify_password, DUMMY_HASH},
    db,
    error::{AppError, AppResult},
    models::User,
    AppState,
};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub user: User,
}

/// Puts `Set-Cookie` headers for a new session of `user` into a header map.
pub fn session_headers(user: &User, state: &AppState) -> AppResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for cookie in session_cookies(user, &state.config)? {
        headers.append(SET_COOKIE, cookie);
    }
    Ok(headers)
}

/// Verifies credentials and starts a cookie session. Failed attempts are rate limited per email.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> AppResult<(HeaderMap, Json<LoginResponse>)> {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || email.len() > 320 {
        return Err(AppError::Unauthorized);
    }
    state
        .login_limiter
        .check(&email)
        .map_err(AppError::TooManyRequests)?;

    let conn = state.db.conn().await?;
    let user: Option<User> =
        db::query_opt(&conn, "SELECT * FROM users WHERE email = ?1", args![&email]).await?;

    // Always run one Argon2 verification so unknown emails take as long as wrong passwords.
    let hash = user
        .as_ref()
        .map_or_else(|| DUMMY_HASH.to_string(), |u| u.password_hash.clone());
    let password_ok = verify_password(body.password, hash).await;

    let Some(user) = user.filter(|_| password_ok) else {
        state.login_limiter.record_failure(&email);
        return Err(AppError::Unauthorized);
    };
    state.login_limiter.reset(&email);
    if !user.is_active {
        return Err(AppError::BadRequest(
            "Your account has been deactivated. Contact an administrator.".into(),
        ));
    }

    let headers = session_headers(&user, &state)?;
    Ok((headers, Json(LoginResponse { user })))
}

/// Ends the session in this browser by expiring the cookies.
pub async fn logout(State(state): State<AppState>) -> (StatusCode, HeaderMap) {
    let mut headers = HeaderMap::new();
    for cookie in clear_cookies(&state.config) {
        headers.append(SET_COOKIE, cookie);
    }
    (StatusCode::NO_CONTENT, headers)
}
