use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_MEMBER: &str = "member";

#[derive(Debug, Clone, Serialize, FromRow)]
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == ROLE_ADMIN
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UserWithStats {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub user: User,
    pub project_count: i64,
    pub total_minutes: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
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

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProjectSummary {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub project: Project,
    pub member_count: i64,
    pub total_minutes: i64,
}

/// A project as seen through one user's membership (admin user-detail page).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UserProject {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub project: Project,
    pub added_at: DateTime<Utc>,
    pub total_minutes: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
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

#[derive(Debug, Clone, Serialize, FromRow)]
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
