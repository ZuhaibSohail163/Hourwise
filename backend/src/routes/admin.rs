use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
    Json, Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use libsql::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    args,
    auth::{hash_password, validate_password, AdminUser},
    db,
    error::{AppError, AppResult},
    models::{
        Project, ProjectMember, ProjectSummary, TimeEntryView, User, UserProject, UserWithStats,
        ENTRY_SELECT, ROLE_ADMIN, ROLE_MEMBER,
    },
    routes::{clean, date_range, valid_color, valid_email},
    AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stats", get(stats))
        .route("/users", get(list_users).post(create_user))
        .route(
            "/users/{id}",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{id}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route("/projects/{id}/members", axum::routing::post(add_members))
        .route("/projects/{id}/members/{user_id}", delete(remove_member))
        .route("/time-entries", get(list_entries))
}

fn email_conflict(e: AppError) -> AppError {
    match e {
        AppError::Conflict(_) => AppError::Conflict("A user with that email already exists".into()),
        other => other,
    }
}

async fn exists(conn: &Connection, sql: &str, id: Uuid) -> AppResult<bool> {
    Ok(db::query_value(conn, sql, args![id]).await?.is_some())
}

// ---------------------------------------------------------------- stats

#[derive(Serialize)]
pub struct Stats {
    users: i64,
    active_users: i64,
    projects: i64,
    active_projects: i64,
    minutes_today: i64,
    minutes_this_week: i64,
    minutes_this_month: i64,
}

/// Start of the (Monday-based) week and of the month containing `today`.
pub fn period_starts(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let week_start = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let month_start = today.with_day(1).unwrap_or(today);
    (week_start, month_start)
}

async fn stats(State(state): State<AppState>, _admin: AdminUser) -> AppResult<Json<Stats>> {
    let today = Utc::now().date_naive();
    let (week_start, month_start) = period_starts(today);
    let conn = state.db.conn().await?;

    let users = db::query_records(
        &conn,
        "SELECT COUNT(*) AS total, COALESCE(SUM(is_active), 0) AS active FROM users",
        args![],
    )
    .await?;
    let projects = db::query_records(
        &conn,
        "SELECT COUNT(*) AS total, COALESCE(SUM(1 - is_archived), 0) AS active FROM projects",
        args![],
    )
    .await?;
    let minutes = db::query_records(
        &conn,
        "SELECT
            COALESCE(SUM(CASE WHEN entry_date = ?1 THEN minutes END), 0) AS today,
            COALESCE(SUM(CASE WHEN entry_date >= ?2 THEN minutes END), 0) AS week,
            COALESCE(SUM(CASE WHEN entry_date >= ?3 THEN minutes END), 0) AS month
         FROM time_entries WHERE entry_date >= ?4 AND entry_date <= ?1",
        args![today, week_start, month_start, week_start.min(month_start)],
    )
    .await?;
    let (Some(u), Some(p), Some(m)) = (users.first(), projects.first(), minutes.first()) else {
        return Err(AppError::Internal(
            "aggregate query returned no rows".into(),
        ));
    };

    Ok(Json(Stats {
        users: u.i64("total")?,
        active_users: u.i64("active")?,
        projects: p.i64("total")?,
        active_projects: p.i64("active")?,
        minutes_today: m.i64("today")?,
        minutes_this_week: m.i64("week")?,
        minutes_this_month: m.i64("month")?,
    }))
}

// ---------------------------------------------------------------- users

const USER_STATS_SELECT: &str = "
SELECT u.*,
    (SELECT COUNT(*) FROM project_members pm WHERE pm.user_id = u.id) AS project_count,
    (SELECT COALESCE(SUM(te.minutes), 0) FROM time_entries te WHERE te.user_id = u.id) AS total_minutes
FROM users u
";

async fn list_users(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<UserWithStats>>> {
    let conn = state.db.conn().await?;
    let sql = format!("{USER_STATS_SELECT} ORDER BY u.is_active DESC, lower(u.name)");
    Ok(Json(db::query_all(&conn, &sql, args![]).await?))
}

pub fn validate_role(role: &str) -> AppResult<()> {
    if role != ROLE_ADMIN && role != ROLE_MEMBER {
        return Err(AppError::bad("Role must be `admin` or `member`"));
    }
    Ok(())
}

pub fn validate_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::bad("Name must be between 1 and 100 characters"));
    }
    Ok(name)
}

pub fn normalize_email(email: &str) -> AppResult<String> {
    let email = email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(AppError::bad("Please enter a valid email address"));
    }
    Ok(email)
}

#[derive(Deserialize)]
pub struct CreateUser {
    email: String,
    name: String,
    password: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    job_title: Option<String>,
    /// Optionally add the new user to these projects right away.
    #[serde(default)]
    project_ids: Vec<Uuid>,
}

async fn create_user(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateUser>,
) -> AppResult<(StatusCode, Json<User>)> {
    let email = normalize_email(&body.email)?;
    let name = validate_name(&body.name)?;
    let role = body.role.unwrap_or_else(|| ROLE_MEMBER.into());
    validate_role(&role)?;
    validate_password(&body.password)?;
    let hash = hash_password(body.password).await?;
    let now = db::now_ts();

    let conn = state.db.conn().await?;
    let tx = db::write_tx(&conn).await?;
    let user: User = db::query_one(
        &tx,
        "INSERT INTO users (id, email, name, password_hash, role, job_title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) RETURNING *",
        args![
            Uuid::new_v4(),
            &email,
            name,
            hash,
            role,
            clean(body.job_title),
            &now
        ],
    )
    .await
    .map_err(email_conflict)?;
    for project_id in body.project_ids {
        if !exists(&tx, "SELECT 1 FROM projects WHERE id = ?1", project_id).await? {
            return Err(AppError::bad("Referenced record does not exist"));
        }
        db::execute(
            &tx,
            "INSERT INTO project_members (project_id, user_id, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            args![project_id, user.id, &now],
        )
        .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(user)))
}

#[derive(Serialize)]
pub struct UserDetail {
    user: UserWithStats,
    projects: Vec<UserProject>,
}

async fn get_user(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<UserDetail>> {
    let conn = state.db.conn().await?;
    let sql = format!("{USER_STATS_SELECT} WHERE u.id = ?1");
    let user: UserWithStats = db::query_opt(&conn, &sql, args![id])
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;

    let projects = db::query_all(
        &conn,
        "SELECT p.*, pm.added_at,
            (SELECT COALESCE(SUM(te.minutes), 0) FROM time_entries te
              WHERE te.project_id = p.id AND te.user_id = ?1) AS total_minutes
         FROM project_members pm
         JOIN projects p ON p.id = pm.project_id
         WHERE pm.user_id = ?1
         ORDER BY p.is_archived, lower(p.name)",
        args![id],
    )
    .await?;

    Ok(Json(UserDetail { user, projects }))
}

#[derive(Deserialize)]
pub struct UpdateUser {
    email: Option<String>,
    name: Option<String>,
    role: Option<String>,
    job_title: Option<String>,
    is_active: Option<bool>,
    /// Admin password reset; the user can change it afterwards from their profile.
    password: Option<String>,
}

async fn update_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUser>,
) -> AppResult<Json<User>> {
    let conn = state.db.conn().await?;
    let existing: User = db::query_opt(&conn, "SELECT * FROM users WHERE id = ?1", args![id])
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;

    let email = match body.email {
        Some(e) => normalize_email(&e)?,
        None => existing.email.clone(),
    };
    let name = match body.name {
        Some(n) => validate_name(&n)?,
        None => existing.name.clone(),
    };
    let role = body.role.unwrap_or_else(|| existing.role.clone());
    validate_role(&role)?;
    let is_active = body.is_active.unwrap_or(existing.is_active);
    let job_title = match body.job_title {
        Some(t) => clean(Some(t)),
        None => existing.job_title.clone(),
    };

    if admin.id == id && (role != ROLE_ADMIN || !is_active) {
        return Err(AppError::bad(
            "You cannot remove your own admin role or deactivate yourself",
        ));
    }

    let password_reset = body.password.as_deref().is_some_and(|p| !p.is_empty());
    let password_hash = match body.password {
        Some(p) if !p.is_empty() => {
            validate_password(&p)?;
            hash_password(p).await?
        }
        _ => existing.password_hash.clone(),
    };
    // A password reset or deactivation signs the user out everywhere.
    let revoke = password_reset || (existing.is_active && !is_active);
    let session_epoch = existing.session_epoch + i64::from(revoke);

    let updated = db::query_one(
        &conn,
        "UPDATE users
         SET email = ?2, name = ?3, role = ?4, is_active = ?5, job_title = ?6,
             password_hash = ?7, updated_at = ?8, session_epoch = ?9
         WHERE id = ?1 RETURNING *",
        args![
            id,
            email,
            name,
            role,
            is_active,
            job_title,
            password_hash,
            db::now_ts(),
            session_epoch
        ],
    )
    .await
    .map_err(email_conflict)?;
    Ok(Json(updated))
}

/// Deletes a user and everything that belongs to them. The cascade is done explicitly (not via
/// `ON DELETE CASCADE`) so it behaves the same on Turso Cloud, where the app does not depend on
/// `PRAGMA foreign_keys` being enabled for the connection.
async fn delete_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    if admin.id == id {
        return Err(AppError::bad("You cannot delete your own account"));
    }
    let conn = state.db.conn().await?;
    let tx = db::write_tx(&conn).await?;
    db::execute(
        &tx,
        "DELETE FROM time_entries WHERE user_id = ?1",
        args![id],
    )
    .await?;
    db::execute(
        &tx,
        "DELETE FROM project_members WHERE user_id = ?1",
        args![id],
    )
    .await?;
    let affected = db::execute(&tx, "DELETE FROM users WHERE id = ?1", args![id]).await?;
    if affected == 0 {
        return Err(AppError::not_found("User"));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- projects

const PROJECT_SUMMARY_SELECT: &str = "
SELECT p.*,
    (SELECT COUNT(*) FROM project_members pm WHERE pm.project_id = p.id) AS member_count,
    (SELECT COALESCE(SUM(te.minutes), 0) FROM time_entries te WHERE te.project_id = p.id) AS total_minutes
FROM projects p
";

async fn list_projects(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<ProjectSummary>>> {
    let conn = state.db.conn().await?;
    let sql = format!("{PROJECT_SUMMARY_SELECT} ORDER BY p.is_archived, lower(p.name)");
    Ok(Json(db::query_all(&conn, &sql, args![]).await?))
}

#[derive(Deserialize)]
pub struct ProjectInput {
    name: Option<String>,
    client: Option<String>,
    description: Option<String>,
    color: Option<String>,
    is_archived: Option<bool>,
}

pub fn validate_project_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::bad(
            "Project name must be between 1 and 120 characters",
        ));
    }
    Ok(name)
}

pub fn validate_project_color(color: Option<String>, fallback: &str) -> AppResult<String> {
    let color = color.unwrap_or_else(|| fallback.to_string()).to_lowercase();
    if !valid_color(&color) {
        return Err(AppError::bad("Color must be a hex value like #f36c00"));
    }
    Ok(color)
}

async fn create_project(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<ProjectInput>,
) -> AppResult<(StatusCode, Json<Project>)> {
    let name = validate_project_name(body.name.as_deref().unwrap_or(""))?;
    let color = validate_project_color(body.color, "#f36c00")?;
    let conn = state.db.conn().await?;
    let project = db::query_one(
        &conn,
        "INSERT INTO projects (id, name, client, description, color, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) RETURNING *",
        args![
            Uuid::new_v4(),
            name,
            clean(body.client),
            clean(body.description),
            color,
            db::now_ts()
        ],
    )
    .await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[derive(Serialize)]
pub struct ProjectDetail {
    project: ProjectSummary,
    members: Vec<ProjectMember>,
}

async fn get_project(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ProjectDetail>> {
    let conn = state.db.conn().await?;
    let sql = format!("{PROJECT_SUMMARY_SELECT} WHERE p.id = ?1");
    let project: ProjectSummary = db::query_opt(&conn, &sql, args![id])
        .await?
        .ok_or_else(|| AppError::not_found("Project"))?;

    let members = db::query_all(
        &conn,
        "SELECT u.id, u.name, u.email, u.avatar_url, u.role, u.job_title, u.is_active, pm.added_at,
            (SELECT COALESCE(SUM(te.minutes), 0) FROM time_entries te
              WHERE te.user_id = u.id AND te.project_id = ?1) AS total_minutes
         FROM project_members pm
         JOIN users u ON u.id = pm.user_id
         WHERE pm.project_id = ?1
         ORDER BY lower(u.name)",
        args![id],
    )
    .await?;

    Ok(Json(ProjectDetail { project, members }))
}

async fn update_project(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ProjectInput>,
) -> AppResult<Json<Project>> {
    let conn = state.db.conn().await?;
    let existing: Project = db::query_opt(&conn, "SELECT * FROM projects WHERE id = ?1", args![id])
        .await?
        .ok_or_else(|| AppError::not_found("Project"))?;

    let name = match body.name {
        Some(n) => validate_project_name(&n)?,
        None => existing.name.clone(),
    };
    let color = validate_project_color(body.color, &existing.color)?;
    let client = match body.client {
        Some(c) => clean(Some(c)),
        None => existing.client.clone(),
    };
    let description = match body.description {
        Some(d) => clean(Some(d)),
        None => existing.description.clone(),
    };
    let is_archived = body.is_archived.unwrap_or(existing.is_archived);

    let project = db::query_one(
        &conn,
        "UPDATE projects
         SET name = ?2, client = ?3, description = ?4, color = ?5, is_archived = ?6, updated_at = ?7
         WHERE id = ?1 RETURNING *",
        args![
            id,
            name,
            client,
            description,
            color,
            is_archived,
            db::now_ts()
        ],
    )
    .await?;
    Ok(Json(project))
}

/// Deletes a project together with its memberships and time entries (explicit cascade, see
/// [`delete_user`]).
async fn delete_project(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let conn = state.db.conn().await?;
    let tx = db::write_tx(&conn).await?;
    db::execute(
        &tx,
        "DELETE FROM time_entries WHERE project_id = ?1",
        args![id],
    )
    .await?;
    db::execute(
        &tx,
        "DELETE FROM project_members WHERE project_id = ?1",
        args![id],
    )
    .await?;
    let affected = db::execute(&tx, "DELETE FROM projects WHERE id = ?1", args![id]).await?;
    if affected == 0 {
        return Err(AppError::not_found("Project"));
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AddMembers {
    user_ids: Vec<Uuid>,
}

async fn add_members(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<AddMembers>,
) -> AppResult<StatusCode> {
    let conn = state.db.conn().await?;
    if !exists(&conn, "SELECT 1 FROM projects WHERE id = ?1", id).await? {
        return Err(AppError::not_found("Project"));
    }
    let now = db::now_ts();
    let tx = db::write_tx(&conn).await?;
    for user_id in body.user_ids {
        if !exists(&tx, "SELECT 1 FROM users WHERE id = ?1", user_id).await? {
            return Err(AppError::bad("Referenced record does not exist"));
        }
        db::execute(
            &tx,
            "INSERT INTO project_members (project_id, user_id, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            args![id, user_id, &now],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a member. Their previously logged time is kept for reporting.
async fn remove_member(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    let conn = state.db.conn().await?;
    db::execute(
        &conn,
        "DELETE FROM project_members WHERE project_id = ?1 AND user_id = ?2",
        args![id, user_id],
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- reports

#[derive(Deserialize)]
pub struct AdminEntryQuery {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    project_id: Option<Uuid>,
    user_id: Option<Uuid>,
}

async fn list_entries(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AdminEntryQuery>,
) -> AppResult<Json<Vec<TimeEntryView>>> {
    let (from, to) = date_range(q.from, q.to)?;
    let sql = format!(
        "{ENTRY_SELECT}
         WHERE te.entry_date BETWEEN ?1 AND ?2
           AND (?3 IS NULL OR te.project_id = ?3)
           AND (?4 IS NULL OR te.user_id = ?4)
         ORDER BY te.entry_date DESC, lower(u.name), te.created_at DESC"
    );
    let conn = state.db.conn().await?;
    let entries = db::query_all(&conn, &sql, args![from, to, q.project_id, q.user_id]).await?;
    Ok(Json(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_default()
    }

    #[test]
    fn weeks_start_on_monday_and_months_on_the_first() {
        // 2026-11-09 is a Monday.
        assert_eq!(
            period_starts(day("2026-11-09")),
            (day("2026-11-09"), day("2026-11-01"))
        );
        // Sunday belongs to the week that started the previous Monday.
        assert_eq!(
            period_starts(day("2026-11-15")),
            (day("2026-11-09"), day("2026-11-01"))
        );
        // A week can start in the previous month (and year).
        assert_eq!(
            period_starts(day("2027-01-01")),
            (day("2026-12-28"), day("2027-01-01"))
        );
        assert_eq!(
            period_starts(day("2024-02-29")),
            (day("2024-02-26"), day("2024-02-01"))
        );
    }

    #[test]
    fn user_field_validation() {
        assert!(validate_role("admin").is_ok());
        assert!(validate_role("member").is_ok());
        assert!(validate_role("Admin").is_err());
        assert!(validate_role("owner").is_err());
        assert_eq!(validate_name("  Ada  ").ok().as_deref(), Some("Ada"));
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(101)).is_err());
        assert_eq!(
            normalize_email("  Ada@Example.COM ").ok().as_deref(),
            Some("ada@example.com")
        );
        assert!(normalize_email("not-an-email").is_err());
    }

    #[test]
    fn project_field_validation() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name(&"p".repeat(121)).is_err());
        assert_eq!(
            validate_project_color(Some("#ABCDEF".into()), "#000000")
                .ok()
                .as_deref(),
            Some("#abcdef")
        );
        assert_eq!(
            validate_project_color(None, "#f36c00").ok().as_deref(),
            Some("#f36c00")
        );
        assert!(validate_project_color(Some("red".into()), "#000000").is_err());
        assert!(validate_project_color(Some("#12345g".into()), "#000000").is_err());
    }
}
