use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, NaiveDate, Utc};
use libsql::{Connection, Value};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    args,
    auth::AuthUser,
    db,
    error::{AppError, AppResult},
    models::{TimeEntryView, ENTRY_SELECT},
    routes::date_range,
    AppState,
};

pub const MAX_DESCRIPTION: usize = 5000;
pub const MINUTES_PER_DAY: i64 = 24 * 60;

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
         WHERE te.user_id = ?1 AND te.entry_date BETWEEN ?2 AND ?3
           AND (?4 IS NULL OR te.project_id = ?4)
         ORDER BY te.entry_date DESC, te.created_at DESC"
    );
    let conn = state.db.conn().await?;
    let entries = db::query_all(&conn, &sql, args![user.id, from, to, q.project_id]).await?;
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

/// Input checks that don't need the database.
pub fn validate_input(input: &EntryInput, today: NaiveDate) -> AppResult<()> {
    if input.minutes <= 0 {
        return Err(AppError::bad("Time must be greater than zero"));
    }
    if i64::from(input.minutes) > MINUTES_PER_DAY {
        return Err(AppError::bad("A single entry cannot exceed 24 hours"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION {
        return Err(AppError::bad(
            "Description is too long (max 5000 characters)",
        ));
    }
    // One day of slack so users ahead of UTC can log "today" in their own time zone.
    if input.entry_date > today + Duration::days(1) {
        return Err(AppError::bad("You cannot log time in the future"));
    }
    Ok(())
}

/// `true` when adding `minutes` to a day that already has `existing` minutes stays within 24h.
pub fn fits_in_day(existing: i64, minutes: i32) -> bool {
    existing + i64::from(minutes) <= MINUTES_PER_DAY
}

/// Shared validation for create and update. Runs inside the write transaction so the
/// daily-total check and the write are atomic.
async fn validate(
    conn: &Connection,
    user_id: Uuid,
    input: &EntryInput,
    existing_id: Option<Uuid>,
) -> AppResult<()> {
    validate_input(input, Utc::now().date_naive())?;

    // Users may only log time to non-archived projects they are a member of.
    let archived = db::query_value(
        conn,
        "SELECT p.is_archived FROM projects p
         JOIN project_members pm ON pm.project_id = p.id
         WHERE p.id = ?1 AND pm.user_id = ?2",
        args![input.project_id, user_id],
    )
    .await?;
    match archived {
        None => return Err(AppError::bad("You are not a member of that project")),
        Some(Value::Integer(0)) => {}
        Some(_) => return Err(AppError::bad("That project is archived")),
    }

    let day_total = db::query_i64(
        conn,
        "SELECT COALESCE(SUM(minutes), 0) FROM time_entries
         WHERE user_id = ?1 AND entry_date = ?2 AND (?3 IS NULL OR id <> ?3)",
        args![user_id, input.entry_date, existing_id],
    )
    .await?;
    if !fits_in_day(day_total, input.minutes) {
        return Err(AppError::bad("Total time for a day cannot exceed 24 hours"));
    }
    Ok(())
}

async fn fetch_view(conn: &Connection, id: Uuid) -> AppResult<TimeEntryView> {
    let sql = format!("{ENTRY_SELECT} WHERE te.id = ?1");
    db::query_one(conn, &sql, args![id]).await
}

pub async fn create(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(input): Json<EntryInput>,
) -> AppResult<(StatusCode, Json<TimeEntryView>)> {
    validate_input(&input, Utc::now().date_naive())?;
    let conn = state.db.conn().await?;
    let id = Uuid::new_v4();
    let now = db::now_ts();

    let tx = db::write_tx(&conn).await?;
    validate(&tx, user.id, &input, None).await?;
    db::execute(
        &tx,
        "INSERT INTO time_entries
            (id, user_id, project_id, entry_date, minutes, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        args![
            id,
            user.id,
            input.project_id,
            input.entry_date,
            input.minutes,
            input.description.trim(),
            now
        ],
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(fetch_view(&conn, id).await?)))
}

pub async fn update(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<EntryInput>,
) -> AppResult<Json<TimeEntryView>> {
    let conn = state.db.conn().await?;
    let tx = db::write_tx(&conn).await?;
    let owner = db::query_value(
        &tx,
        "SELECT user_id FROM time_entries WHERE id = ?1",
        args![id],
    )
    .await?;
    // Another user's entry is reported as "not found" so ids can't be probed.
    if owner != Some(Value::Text(user.id.hyphenated().to_string())) {
        return Err(AppError::not_found("Time entry"));
    }
    validate(&tx, user.id, &input, Some(id)).await?;
    db::execute(
        &tx,
        "UPDATE time_entries
         SET project_id = ?2, entry_date = ?3, minutes = ?4, description = ?5, updated_at = ?6
         WHERE id = ?1",
        args![
            id,
            input.project_id,
            input.entry_date,
            input.minutes,
            input.description.trim(),
            db::now_ts()
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(fetch_view(&conn, id).await?))
}

pub async fn remove(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let conn = state.db.conn().await?;
    let affected = db::execute(
        &conn,
        "DELETE FROM time_entries WHERE id = ?1 AND user_id = ?2",
        args![id, user.id],
    )
    .await?;
    if affected == 0 {
        return Err(AppError::not_found("Time entry"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(minutes: i32, date: NaiveDate, desc: &str) -> EntryInput {
        EntryInput {
            project_id: Uuid::nil(),
            entry_date: date,
            minutes,
            description: desc.into(),
        }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_default()
    }

    #[test]
    fn minutes_must_be_positive_and_at_most_a_day() {
        let today = day("2026-11-09");
        assert!(validate_input(&input(0, today, ""), today).is_err());
        assert!(validate_input(&input(-5, today, ""), today).is_err());
        assert!(validate_input(&input(1, today, ""), today).is_ok());
        assert!(validate_input(&input(1440, today, ""), today).is_ok());
        assert!(validate_input(&input(1441, today, ""), today).is_err());
    }

    #[test]
    fn future_dates_allow_one_day_of_slack() {
        let today = day("2026-12-31");
        assert!(validate_input(&input(60, day("2027-01-01"), ""), today).is_ok());
        assert!(validate_input(&input(60, day("2027-01-02"), ""), today).is_err());
        assert!(validate_input(&input(60, day("2020-02-29"), ""), today).is_ok());
    }

    #[test]
    fn description_limit_counts_characters_not_bytes() {
        let today = day("2026-11-09");
        let ok = "\u{e9}".repeat(MAX_DESCRIPTION);
        assert!(validate_input(&input(60, today, &ok), today).is_ok());
        let too_long = "a".repeat(MAX_DESCRIPTION + 1);
        assert!(validate_input(&input(60, today, &too_long), today).is_err());
    }

    #[test]
    fn daily_cap_is_inclusive_24_hours() {
        assert!(fits_in_day(0, 1440));
        assert!(fits_in_day(1380, 60));
        assert!(!fits_in_day(1380, 61));
        assert!(!fits_in_day(1440, 1));
    }
}
