use axum::{extract::State, Json};

use crate::{args, auth::AuthUser, db, error::AppResult, models::Project, AppState};

/// Projects the signed-in user is a member of (the only ones they can log time to).
pub async fn my_projects(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> AppResult<Json<Vec<Project>>> {
    let mut conn = state.db.conn().await?;
    let projects = db::query_all(
        &mut conn,
        "SELECT p.* FROM projects p
         JOIN project_members pm ON pm.project_id = p.id
         WHERE pm.user_id = $1
         ORDER BY p.is_archived, lower(p.name)",
        args![user.id],
    )
    .await?;
    Ok(Json(projects))
}
