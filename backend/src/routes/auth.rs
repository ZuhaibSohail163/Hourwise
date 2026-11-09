use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::{
    auth::{create_token, verify_password},
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
    pub token: String,
    pub user: User,
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> AppResult<Json<LoginResponse>> {
    let email = body.email.trim().to_lowercase();
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if !verify_password(body.password, user.password_hash.clone()).await {
        return Err(AppError::Unauthorized);
    }
    if !user.is_active {
        return Err(AppError::BadRequest(
            "Your account has been deactivated. Contact an administrator.".into(),
        ));
    }

    let token = create_token(&user, &state.config.jwt_secret, state.config.jwt_ttl_hours)?;
    Ok(Json(LoginResponse { token, user }))
}
