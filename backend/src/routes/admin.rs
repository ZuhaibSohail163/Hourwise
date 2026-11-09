use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
    Json, Router,
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{hash_password, validate_password, AdminUser},
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

async fn stats(State(state): State<AppState>, _admin: AdminUser) -> AppResult<Json<Stats>> {
    let today = Utc::now().date_naive();
    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let month_start = today.with_day(1).unwrap_or(today);

    let (users, active_users): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COUNT(*) FILTER (WHERE is_active) FROM users")
            .fetch_one(&state.db)
            .await?;
    let (projects, active_projects): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COUNT(*) FILTER (WHERE NOT is_archived) FROM projects")
            .fetch_one(&state.db)
            .await?;
    let (minutes_today, minutes_this_week, minutes_this_month): (i64, i64, i64) = sqlx::query_as(
        "SELECT
            COALESCE(SUM(minutes) FILTER (WHERE entry_date = $1), 0)::BIGINT,
            COALESCE(SUM(minutes) FILTER (WHERE entry_date >= $2), 0)::BIGINT,
            COALESCE(SUM(minutes) FILTER (WHERE entry_date >= $3), 0)::BIGINT
         FROM time_entries WHERE entry_date >= LEAST($2, $3) AND entry_date <= $1",
    )
    .bind(today)
    .bind(week_start)
    .bind(month_start)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(Stats {
        users,
        active_users,
        projects,
        active_projects,
        minutes_today,
        minutes_this_week,
        minutes_this_month,
    }))
}

// ---------------------------------------------------------------- users

async fn list_users(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<UserWithStats>>> {
    let users = sqlx::query_as::<_, UserWithStats>(
        "SELECT u.*,
            (SELECT COUNT(*) FROM project_members pm WHERE pm.user_id = u.id) AS project_count,
            (SELECT COALESCE(SUM(te.minutes), 0)::BIGINT FROM time_entries te WHERE te.user_id = u.id) AS total_minutes
         FROM users u
         ORDER BY u.is_active DESC, lower(u.name)",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

fn validate_role(role: &str) -> AppResult<()> {
    if role != ROLE_ADMIN && role != ROLE_MEMBER {
        return Err(AppError::bad("Role must be `admin` or `member`"));
    }
    Ok(())
}

fn validate_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::bad("Name must be between 1 and 100 characters"));
    }
    Ok(name)
}

fn normalize_email(email: &str) -> AppResult<String> {
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

    let mut tx = state.db.begin().await?;
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (id, email, name, password_hash, role, job_title)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(&email)
    .bind(name)
    .bind(hash)
    .bind(role)
    .bind(clean(body.job_title))
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match AppError::from(e) {
        AppError::Conflict(_) => AppError::Conflict("A user with that email already exists".into()),
        other => other,
    })?;
    for project_id in body.project_ids {
        sqlx::query(
            "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(project_id)
        .bind(user.id)
        .execute(&mut *tx)
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
    let user = sqlx::query_as::<_, UserWithStats>(
        "SELECT u.*,
            (SELECT COUNT(*) FROM project_members pm WHERE pm.user_id = u.id) AS project_count,
            (SELECT COALESCE(SUM(te.minutes), 0)::BIGINT FROM time_entries te WHERE te.user_id = u.id) AS total_minutes
         FROM users u WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("User"))?;

    let projects = sqlx::query_as::<_, UserProject>(
        "SELECT p.*, pm.added_at,
            (SELECT COALESCE(SUM(te.minutes), 0)::BIGINT FROM time_entries te
              WHERE te.project_id = p.id AND te.user_id = $1) AS total_minutes
         FROM project_members pm
         JOIN projects p ON p.id = pm.project_id
         WHERE pm.user_id = $1
         ORDER BY p.is_archived, lower(p.name)",
    )
    .bind(id)
    .fetch_all(&state.db)
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
    let existing = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
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

    let password_hash = match body.password {
        Some(p) if !p.is_empty() => {
            validate_password(&p)?;
            hash_password(p).await?
        }
        _ => existing.password_hash.clone(),
    };

    let updated = sqlx::query_as::<_, User>(
        "UPDATE users
         SET email = $2, name = $3, role = $4, is_active = $5, job_title = $6,
             password_hash = $7, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(email)
    .bind(name)
    .bind(role)
    .bind(is_active)
    .bind(job_title)
    .bind(password_hash)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match AppError::from(e) {
        AppError::Conflict(_) => AppError::Conflict("A user with that email already exists".into()),
        other => other,
    })?;
    Ok(Json(updated))
}

async fn delete_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    if admin.id == id {
        return Err(AppError::bad("You cannot delete your own account"));
    }
    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("User"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- projects

const PROJECT_SUMMARY_SELECT: &str = "
SELECT p.*,
    (SELECT COUNT(*) FROM project_members pm WHERE pm.project_id = p.id) AS member_count,
    (SELECT COALESCE(SUM(te.minutes), 0)::BIGINT FROM time_entries te WHERE te.project_id = p.id) AS total_minutes
FROM projects p
";

async fn list_projects(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> AppResult<Json<Vec<ProjectSummary>>> {
    let sql = format!("{PROJECT_SUMMARY_SELECT} ORDER BY p.is_archived, lower(p.name)");
    let projects = sqlx::query_as::<_, ProjectSummary>(&sql)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(projects))
}

#[derive(Deserialize)]
pub struct ProjectInput {
    name: Option<String>,
    client: Option<String>,
    description: Option<String>,
    color: Option<String>,
    is_archived: Option<bool>,
}

fn validate_project_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::bad(
            "Project name must be between 1 and 120 characters",
        ));
    }
    Ok(name)
}

fn validate_project_color(color: Option<String>, fallback: &str) -> AppResult<String> {
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
    let project = sqlx::query_as::<_, Project>(
        "INSERT INTO projects (id, name, client, description, color)
         VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(name)
    .bind(clean(body.client))
    .bind(clean(body.description))
    .bind(color)
    .fetch_one(&state.db)
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
    let sql = format!("{PROJECT_SUMMARY_SELECT} WHERE p.id = $1");
    let project = sqlx::query_as::<_, ProjectSummary>(&sql)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("Project"))?;

    let members = sqlx::query_as::<_, ProjectMember>(
        "SELECT u.id, u.name, u.email, u.avatar_url, u.role, u.job_title, u.is_active, pm.added_at,
            (SELECT COALESCE(SUM(te.minutes), 0)::BIGINT FROM time_entries te
              WHERE te.user_id = u.id AND te.project_id = $1) AS total_minutes
         FROM project_members pm
         JOIN users u ON u.id = pm.user_id
         WHERE pm.project_id = $1
         ORDER BY lower(u.name)",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(ProjectDetail { project, members }))
}

async fn update_project(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ProjectInput>,
) -> AppResult<Json<Project>> {
    let existing = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
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

    let project = sqlx::query_as::<_, Project>(
        "UPDATE projects
         SET name = $2, client = $3, description = $4, color = $5, is_archived = $6, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(client)
    .bind(description)
    .bind(color)
    .bind(is_archived)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(project))
}

async fn delete_project(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let result = sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Project"));
    }
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
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM projects WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::not_found("Project"));
    }
    let mut tx = state.db.begin().await?;
    for user_id in body.user_ids {
        sqlx::query(
            "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
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
    sqlx::query("DELETE FROM project_members WHERE project_id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
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
         WHERE te.entry_date BETWEEN $1 AND $2
           AND ($3::uuid IS NULL OR te.project_id = $3)
           AND ($4::uuid IS NULL OR te.user_id = $4)
         ORDER BY te.entry_date DESC, lower(u.name), te.created_at DESC"
    );
    let entries = sqlx::query_as::<_, TimeEntryView>(&sql)
        .bind(from)
        .bind(to)
        .bind(q.project_id)
        .bind(q.user_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(entries))
}
