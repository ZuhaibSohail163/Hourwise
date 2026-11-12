use std::path::Path;

use axum::{
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    args,
    auth::{validate_password, AuthUser},
    db,
    error::{AppError, AppResult},
    models::User,
    routes::{auth::session_headers, clean},
    AppState,
};

const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

pub async fn get_me(AuthUser(user): AuthUser) -> Json<User> {
    Json(user)
}

#[derive(Deserialize)]
pub struct UpdateMe {
    pub name: Option<String>,
    pub job_title: Option<String>,
}

/// Users can change their own name and job title. Email is admin-only.
pub async fn update_me(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(body): Json<UpdateMe>,
) -> AppResult<Json<User>> {
    let name = match body.name {
        Some(n) => {
            let n = n.trim().to_string();
            if n.is_empty() || n.chars().count() > 100 {
                return Err(AppError::bad("Name must be between 1 and 100 characters"));
            }
            n
        }
        None => user.name.clone(),
    };
    let job_title = match body.job_title {
        Some(t) => clean(Some(t)),
        None => user.job_title.clone(),
    };
    let mut conn = state.db.conn().await?;
    let updated: User = db::query_one(
        &mut conn,
        "UPDATE users SET name = $2, job_title = $3, updated_at = $4 WHERE id = $1 RETURNING *",
        args![user.id, name, job_title, db::now_ts()],
    )
    .await?;
    Ok(Json(updated))
}

#[derive(Deserialize)]
pub struct ChangePassword {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(body): Json<ChangePassword>,
) -> AppResult<(StatusCode, HeaderMap)> {
    if !state
        .hashing
        .verify(body.current_password, user.password_hash.clone())
        .await?
    {
        return Err(AppError::bad("Current password is incorrect"));
    }
    validate_password(&body.new_password)?;
    let hash = state.hashing.hash(body.new_password).await?;
    let mut conn = state.db.conn().await?;
    // Bumping the epoch signs out every other browser/device; this one gets a fresh session.
    let updated: User = db::query_one(
        &mut conn,
        "UPDATE users SET password_hash = $2, session_epoch = session_epoch + 1, updated_at = $3
         WHERE id = $1 RETURNING *",
        args![user.id, hash, db::now_ts()],
    )
    .await?;
    Ok((StatusCode::NO_CONTENT, session_headers(&updated, &state)?))
}

fn extension_for(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// Magic-byte check so a renamed non-image can't be stored as an avatar.
fn looks_like_image(ext: &str, data: &[u8]) -> bool {
    match ext {
        "png" => data.starts_with(&[0x89, b'P', b'N', b'G']),
        "jpg" => data.starts_with(&[0xFF, 0xD8, 0xFF]),
        "gif" => data.starts_with(b"GIF8"),
        "webp" => data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP",
        _ => false,
    }
}

async fn remove_avatar_file(upload_dir: &str, avatar_url: &Option<String>) {
    if let Some(url) = avatar_url {
        if let Some(file) = url.strip_prefix("/uploads/avatars/") {
            // `file` is always a name we generated, but never allow traversal.
            if !file.contains('/') && !file.contains('\\') && !file.contains("..") {
                let _ =
                    tokio::fs::remove_file(Path::new(upload_dir).join("avatars").join(file)).await;
            }
        }
    }
}

pub async fn upload_avatar(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    mut multipart: Multipart,
) -> AppResult<Json<User>> {
    let mut upload: Option<(&'static str, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad(format!("Invalid upload: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let ext = field
            .content_type()
            .and_then(extension_for)
            .ok_or_else(|| AppError::bad("Avatar must be a PNG, JPEG, WEBP or GIF image"))?;
        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::bad(format!("Invalid upload: {e}")))?;
        if data.len() > MAX_AVATAR_BYTES {
            return Err(AppError::bad("Avatar must be 2 MB or smaller"));
        }
        if !looks_like_image(ext, &data) {
            return Err(AppError::bad("File does not look like a valid image"));
        }
        upload = Some((ext, data.to_vec()));
    }
    let (ext, data) = upload.ok_or_else(|| AppError::bad("No file was uploaded (field `file`)"))?;

    let filename = format!("{}-{}.{}", user.id, Uuid::new_v4().simple(), ext);
    let path = Path::new(&state.config.upload_dir)
        .join("avatars")
        .join(&filename);
    tokio::fs::write(&path, &data)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut conn = state.db.conn().await?;
    let updated: User = db::query_one(
        &mut conn,
        "UPDATE users SET avatar_url = $2, updated_at = $3 WHERE id = $1 RETURNING *",
        args![
            user.id,
            format!("/uploads/avatars/{filename}"),
            db::now_ts()
        ],
    )
    .await?;

    remove_avatar_file(&state.config.upload_dir, &user.avatar_url).await;
    Ok(Json(updated))
}

pub async fn delete_avatar(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> AppResult<Json<User>> {
    let mut conn = state.db.conn().await?;
    let updated: User = db::query_one(
        &mut conn,
        "UPDATE users SET avatar_url = NULL, updated_at = $2 WHERE id = $1 RETURNING *",
        args![user.id, db::now_ts()],
    )
    .await?;
    remove_avatar_file(&state.config.upload_dir, &user.avatar_url).await;
    Ok(Json(updated))
}
