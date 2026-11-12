//! Data-integrity report (`hourwise-backend verify`).
//!
//! Read-only: it only runs `SELECT`s, so it is safe against any database, including production.
//! Most rules are also enforced by the schema (foreign keys, `CHECK`s); the report double-checks
//! them and the rules that only the application enforces (the 24h daily cap, an active admin).

use std::collections::BTreeMap;

use crate::{
    args,
    db::{self, Connection, Db},
    error::AppResult,
};

/// Tables in foreign-key order (parents first).
pub const TABLES: &[&str] = &["users", "projects", "project_members", "time_entries"];

pub type Counts = BTreeMap<String, i64>;

pub async fn table_counts(conn: &mut Connection) -> AppResult<Counts> {
    let mut counts = Counts::new();
    for table in TABLES {
        // Table names come from the constant list above, never from input.
        let n = db::query_i64(conn, &format!("SELECT COUNT(*) FROM {table}"), args![]).await?;
        counts.insert((*table).to_string(), n);
    }
    Ok(counts)
}

/// Result of [`verify`].
#[derive(Debug, Default)]
pub struct Report {
    pub counts: Counts,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub async fn verify(db: &Db) -> AppResult<Report> {
    let mut conn = db.conn().await?;
    let mut report = Report {
        counts: table_counts(&mut conn).await?,
        ..Report::default()
    };
    let checks: &[(&str, &str)] = &[
        (
            "memberships pointing at a missing user or project",
            "SELECT COUNT(*) FROM project_members pm
             WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id = pm.user_id)
                OR NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = pm.project_id)",
        ),
        (
            "time entries pointing at a missing user or project",
            "SELECT COUNT(*) FROM time_entries te
             WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.id = te.user_id)
                OR NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = te.project_id)",
        ),
        (
            "time entries outside 1..1440 minutes",
            "SELECT COUNT(*) FROM time_entries WHERE minutes < 1 OR minutes > 1440",
        ),
        (
            "user-days with more than 24 hours logged",
            "SELECT COUNT(*) FROM (SELECT 1 FROM time_entries
             GROUP BY user_id, entry_date HAVING SUM(minutes) > 1440) AS over_cap",
        ),
        (
            "emails that are not lower-case or are duplicated ignoring case",
            "SELECT COUNT(*) FROM users u WHERE u.email <> lower(u.email)
                OR (SELECT COUNT(*) FROM users v WHERE lower(v.email) = lower(u.email)) > 1",
        ),
        (
            "users with an invalid role",
            "SELECT COUNT(*) FROM users WHERE role NOT IN ('admin', 'member')",
        ),
    ];
    for (label, sql) in checks {
        let n = db::query_i64(&mut conn, sql, args![]).await?;
        if n > 0 {
            report.problems.push(format!("{n} {label}"));
        }
    }
    let admins = db::query_i64(
        &mut conn,
        "SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active",
        args![],
    )
    .await?;
    if admins == 0 {
        report.problems.push("no active admin account".into());
    }
    if let Some(version) =
        db::query_scalar::<String>(&mut conn, "SHOW server_version", args![]).await?
    {
        report.notes.push(format!("PostgreSQL {version}"));
    }
    Ok(report)
}
