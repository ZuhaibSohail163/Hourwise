use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    db::{FromRecord, Record},
    error::{AppError, AppResult},
};

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_MEMBER: &str = "member";

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub job_title: Option<String>,
    pub avatar_url: Option<String>,
    pub is_active: bool,
    /// Incremented to revoke every session of this user (see `auth.rs`).
    #[serde(skip_serializing)]
    pub session_epoch: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == ROLE_ADMIN
    }
}

impl FromRecord for User {
    fn from_record(r: &Record) -> AppResult<Self> {
        Ok(User {
            id: r.uuid("id")?,
            email: r.text("email")?,
            name: r.text("name")?,
            password_hash: r.text("password_hash")?,
            role: r.text("role")?,
            job_title: r.opt_text("job_title")?,
            avatar_url: r.opt_text("avatar_url")?,
            is_active: r.bool("is_active")?,
            session_epoch: r.i64("session_epoch")?,
            created_at: r.ts("created_at")?,
            updated_at: r.ts("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UserWithStats {
    #[serde(flatten)]
    pub user: User,
    pub project_count: i64,
    pub total_minutes: i64,
}

impl FromRecord for UserWithStats {
    fn from_record(r: &Record) -> AppResult<Self> {
        Ok(UserWithStats {
            user: User::from_record(r)?,
            project_count: r.i64("project_count")?,
            total_minutes: r.i64("total_minutes")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub client: Option<String>,
    pub description: Option<String>,
    pub color: String,
    pub is_archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FromRecord for Project {
    fn from_record(r: &Record) -> AppResult<Self> {
        Ok(Project {
            id: r.uuid("id")?,
            name: r.text("name")?,
            client: r.opt_text("client")?,
            description: r.opt_text("description")?,
            color: r.text("color")?,
            is_archived: r.bool("is_archived")?,
            created_at: r.ts("created_at")?,
            updated_at: r.ts("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    #[serde(flatten)]
    pub project: Project,
    pub member_count: i64,
    pub total_minutes: i64,
}

impl FromRecord for ProjectSummary {
    fn from_record(r: &Record) -> AppResult<Self> {
        Ok(ProjectSummary {
            project: Project::from_record(r)?,
            member_count: r.i64("member_count")?,
            total_minutes: r.i64("total_minutes")?,
        })
    }
}

/// A project as seen through one user's membership (admin user-detail page).
#[derive(Debug, Clone, Serialize)]
pub struct UserProject {
    #[serde(flatten)]
    pub project: Project,
    pub added_at: DateTime<Utc>,
    pub total_minutes: i64,
}

impl FromRecord for UserProject {
    fn from_record(r: &Record) -> AppResult<Self> {
        Ok(UserProject {
            project: Project::from_record(r)?,
            added_at: r.ts("added_at")?,
            total_minutes: r.i64("total_minutes")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectMember {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub job_title: Option<String>,
    pub is_active: bool,
    pub added_at: DateTime<Utc>,
    pub total_minutes: i64,
}

impl FromRecord for ProjectMember {
    fn from_record(r: &Record) -> AppResult<Self> {
        Ok(ProjectMember {
            id: r.uuid("id")?,
            name: r.text("name")?,
            email: r.text("email")?,
            avatar_url: r.opt_text("avatar_url")?,
            role: r.text("role")?,
            job_title: r.opt_text("job_title")?,
            is_active: r.bool("is_active")?,
            added_at: r.ts("added_at")?,
            total_minutes: r.i64("total_minutes")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeEntryView {
    pub id: Uuid,
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub entry_date: NaiveDate,
    pub minutes: i32,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_name: String,
    pub project_color: String,
    pub project_client: Option<String>,
    pub user_name: String,
    pub user_email: String,
}

impl FromRecord for TimeEntryView {
    fn from_record(r: &Record) -> AppResult<Self> {
        let minutes = i32::try_from(r.i64("minutes")?)
            .map_err(|_| AppError::Internal("minutes out of range".into()))?;
        Ok(TimeEntryView {
            id: r.uuid("id")?,
            user_id: r.uuid("user_id")?,
            project_id: r.uuid("project_id")?,
            entry_date: r.date("entry_date")?,
            minutes,
            description: r.text("description")?,
            created_at: r.ts("created_at")?,
            updated_at: r.ts("updated_at")?,
            project_name: r.text("project_name")?,
            project_color: r.text("project_color")?,
            project_client: r.opt_text("project_client")?,
            user_name: r.text("user_name")?,
            user_email: r.text("user_email")?,
        })
    }
}

/// Base SELECT for `TimeEntryView`; append WHERE / ORDER BY clauses.
pub const ENTRY_SELECT: &str = "
SELECT te.id, te.user_id, te.project_id, te.entry_date, te.minutes, te.description,
       te.created_at, te.updated_at,
       p.name AS project_name, p.color AS project_color, p.client AS project_client,
       u.name AS user_name, u.email AS user_email
FROM time_entries te
JOIN projects p ON p.id = te.project_id
JOIN users u ON u.id = te.user_id
";
