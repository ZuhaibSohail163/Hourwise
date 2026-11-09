use axum::{extract::State, Json};

use crate::{auth::AuthUser, error::AppResult, models::Project, AppState};

/// Projects the signed-in user is a member of (the only ones they can log time to).
pub async fn my_projects(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> AppResult<Json<Vec<Project>>> {
    let projects = sqlx::query_as::<_, Project>(
        "SELECT p.* FROM projects p
         JOIN project_members pm ON pm.project_id = p.id
         WHERE pm.user_id = $1
         ORDER BY p.is_archived, lower(p.name)",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(projects))
}
