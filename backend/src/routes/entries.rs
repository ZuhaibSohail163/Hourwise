use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{AppError, AppResult},
    models::{TimeEntryView, ENTRY_SELECT},
    routes::date_range,
    AppState,
};

const MAX_DESCRIPTION: usize = 5000;
const MINUTES_PER_DAY: i64 = 24 * 60;

#[derive(Deserialize)]
pub struct EntryQuery {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub project_id: Option<Uuid>,
}

pub async fn list(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(q): Query<EntryQuery>,
) -> AppResult<Json<Vec<TimeEntryView>>> {
    let (from, to) = date_range(q.from, q.to)?;
    let sql = format!(
        "{ENTRY_SELECT}
         WHERE te.user_id = $1 AND te.entry_date BETWEEN $2 AND $3
           AND ($4::uuid IS NULL OR te.project_id = $4)
         ORDER BY te.entry_date DESC, te.created_at DESC"
    );
    let entries = sqlx::query_as::<_, TimeEntryView>(&sql)
        .bind(user.id)
        .bind(from)
        .bind(to)
        .bind(q.project_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(entries))
}

#[derive(Deserialize)]
pub struct EntryInput {
    pub project_id: Uuid,
    pub entry_date: NaiveDate,
    pub minutes: i32,
    #[serde(default)]
    pub description: String,
}

/// Shared validation for create and update.
async fn validate(
    db: &PgPool,
    user_id: Uuid,
    input: &EntryInput,
    existing_id: Option<Uuid>,
) -> AppResult<()> {
    if input.minutes <= 0 {
        return Err(AppError::bad("Time must be greater than zero"));
    }
    if input.minutes as i64 > MINUTES_PER_DAY {
        return Err(AppError::bad("A single entry cannot exceed 24 hours"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION {
        return Err(AppError::bad(
            "Description is too long (max 5000 characters)",
        ));
    }
    if input.entry_date > Utc::now().date_naive() + Duration::days(1) {
        return Err(AppError::bad("You cannot log time in the future"));
    }

    // Users may only log time to non-archived projects they are a member of.
    let archived: Option<bool> = sqlx::query_scalar(
        "SELECT p.is_archived FROM projects p
         JOIN project_members pm ON pm.project_id = p.id
         WHERE p.id = $1 AND pm.user_id = $2",
    )
    .bind(input.project_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    match archived {
        None => return Err(AppError::bad("You are not a member of that project")),
        Some(true) => return Err(AppError::bad("That project is archived")),
        Some(false) => {}
    }

    let day_total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(minutes), 0)::BIGINT FROM time_entries
         WHERE user_id = $1 AND entry_date = $2 AND ($3::uuid IS NULL OR id <> $3)",
    )
    .bind(user_id)
    .bind(input.entry_date)
    .bind(existing_id)
    .fetch_one(db)
    .await?;
    if day_total + input.minutes as i64 > MINUTES_PER_DAY {
        return Err(AppError::bad("Total time for a day cannot exceed 24 hours"));
    }
    Ok(())
}

async fn fetch_view(db: &PgPool, id: Uuid) -> AppResult<TimeEntryView> {
    let sql = format!("{ENTRY_SELECT} WHERE te.id = $1");
    Ok(sqlx::query_as::<_, TimeEntryView>(&sql)
        .bind(id)
        .fetch_one(db)
        .await?)
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(input): Json<EntryInput>,
) -> AppResult<(StatusCode, Json<TimeEntryView>)> {
    validate(&state.db, user.id, &input, None).await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes, description)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(user.id)
    .bind(input.project_id)
    .bind(input.entry_date)
    .bind(input.minutes)
    .bind(input.description.trim())
    .execute(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(fetch_view(&state.db, id).await?)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<EntryInput>,
) -> AppResult<Json<TimeEntryView>> {
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM time_entries WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    if owner != Some(user.id) {
        return Err(AppError::not_found("Time entry"));
    }
    validate(&state.db, user.id, &input, Some(id)).await?;
    sqlx::query(
        "UPDATE time_entries
         SET project_id = $2, entry_date = $3, minutes = $4, description = $5, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(input.project_id)
    .bind(input.entry_date)
    .bind(input.minutes)
    .bind(input.description.trim())
    .execute(&state.db)
    .await?;
    Ok(Json(fetch_view(&state.db, id).await?))
}

pub async fn remove(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let result = sqlx::query("DELETE FROM time_entries WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Time entry"));
    }
    Ok(StatusCode::NO_CONTENT)
}
