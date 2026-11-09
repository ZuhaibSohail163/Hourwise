//! Logical backups, restores and integrity checks.
//!
//! `export` writes a plain-text SQL file (one `INSERT` per line) that can be stored anywhere and
//! restored into any empty Hourwise database: Turso Cloud, a new Turso database or a local file.
//! It works with only `TURSO_DATABASE_URL` / `TURSO_AUTH_TOKEN`, so it needs neither the Turso CLI
//! nor a paid plan, and it is independent of Turso's own point-in-time restore.
//!
//! `import` only accepts `INSERT INTO <known table>` statements, runs them in one transaction,
//! and checks that the row counts match the counts recorded in the file header.

use std::{collections::BTreeMap, fmt::Write as _};

use libsql::{Connection, Value};

use crate::{
    args,
    db::{self, Db},
    error::{AppError, AppResult},
    migrate,
};

/// Tables in foreign-key order (parents first).
pub const TABLES: &[&str] = &["users", "projects", "project_members", "time_entries"];
const HEADER: &str = "-- hourwise-backup v1";
const BATCH: usize = 200;

pub type Counts = BTreeMap<String, i64>;

pub async fn table_counts(conn: &Connection) -> AppResult<Counts> {
    let mut counts = Counts::new();
    for table in TABLES {
        // Table names come from the constant list above, never from input.
        let n = db::query_i64(conn, &format!("SELECT COUNT(*) FROM {table}"), args![]).await?;
        counts.insert((*table).to_string(), n);
    }
    Ok(counts)
}

fn format_counts(counts: &Counts) -> String {
    counts
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Renders a value as a SQLite literal that fits on one line.
pub fn sql_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => format!("{f:?}"),
        Value::Text(s) if !s.chars().any(char::is_control) => {
            format!("'{}'", s.replace('\'', "''"))
        }
        // Newlines, tabs etc.: hex-encode so every statement stays on a single line.
        Value::Text(s) => format!("CAST(X'{}' AS TEXT)", hex(s.as_bytes())),
        Value::Blob(b) => format!("X'{}'", hex(b)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

async fn columns(conn: &Connection, table: &str) -> AppResult<Vec<String>> {
    let rows = db::query_records(
        conn,
        "SELECT name FROM pragma_table_info(?1) ORDER BY cid",
        args![table],
    )
    .await?;
    let cols: Vec<String> = rows
        .iter()
        .map(|r| r.text("name"))
        .collect::<AppResult<_>>()?;
    if cols.is_empty() {
        return Err(AppError::Internal(format!("table {table} not found")));
    }
    Ok(cols)
}

/// Produces the full backup as a string.
pub async fn export(db: &Db) -> AppResult<String> {
    let conn = db.conn().await?;
    let version = migrate::applied(&conn)
        .await?
        .last()
        .map(|(v, _)| *v)
        .unwrap_or(0);
    // Read everything inside one transaction so the snapshot is consistent.
    let tx = conn.transaction().await?;
    let counts = table_counts(&tx).await?;
    let mut out = String::new();
    let _ = writeln!(out, "{HEADER}");
    let _ = writeln!(out, "-- created_at: {}", db::now_ts());
    let _ = writeln!(out, "-- schema_version: {version}");
    let _ = writeln!(out, "-- counts: {}", format_counts(&counts));
    for table in TABLES {
        let cols = columns(&tx, table).await?;
        let col_list = cols.join(", ");
        let order = if *table == "project_members" {
            "project_id, user_id"
        } else {
            "id"
        };
        let rows = db::query_records(
            &tx,
            &format!("SELECT {col_list} FROM {table} ORDER BY {order}"),
            args![],
        )
        .await?;
        for row in &rows {
            let values: Vec<String> = (0..cols.len())
                .map(|i| row.raw(i).map(sql_literal).unwrap_or_else(|| "NULL".into()))
                .collect();
            let _ = writeln!(
                out,
                "INSERT INTO {table} ({col_list}) VALUES ({});",
                values.join(", ")
            );
        }
    }
    tx.commit().await?;
    let _ = writeln!(out, "-- end of backup");
    Ok(out)
}

/// `true` when the only `;` outside string literals is the final character, i.e. the line is
/// exactly one statement (so a crafted file cannot smuggle in `; DROP TABLE …`).
pub fn single_statement(line: &str) -> bool {
    let mut in_string = false;
    let mut semicolons = 0;
    for c in line.chars() {
        match c {
            '\x27' => in_string = !in_string,
            ';' if !in_string => semicolons += 1,
            _ => {}
        }
    }
    !in_string && semicolons == 1 && line.ends_with(';')
}

/// Parsed header of a backup file.
#[derive(Debug, PartialEq, Eq)]
pub struct BackupInfo {
    pub schema_version: i64,
    pub counts: Counts,
    pub statements: Vec<String>,
}

pub fn parse(text: &str) -> AppResult<BackupInfo> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some(HEADER) {
        return Err(AppError::bad("Not a Hourwise backup file (missing header)"));
    }
    let mut schema_version = None;
    let mut counts = None;
    let mut statements = Vec::new();
    let mut complete = false;
    for line in lines {
        let line = line.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix("-- schema_version: ") {
            schema_version = v.trim().parse::<i64>().ok();
        } else if let Some(c) = line.strip_prefix("-- counts: ") {
            let mut map = Counts::new();
            for pair in c.split_whitespace() {
                let (k, v) = pair
                    .split_once('=')
                    .ok_or_else(|| AppError::bad("Malformed counts line"))?;
                let v = v
                    .parse::<i64>()
                    .map_err(|_| AppError::bad("Malformed counts line"))?;
                map.insert(k.to_string(), v);
            }
            counts = Some(map);
        } else if line == "-- end of backup" {
            complete = true;
        } else if line.starts_with("--") || line.trim().is_empty() {
            continue;
        } else {
            let allowed = TABLES
                .iter()
                .any(|t| line.starts_with(&format!("INSERT INTO {t} (")));
            if !allowed || !line.ends_with(");") || !single_statement(line) {
                return Err(AppError::bad(format!(
                    "Refusing to restore: unexpected statement `{}`",
                    line.chars().take(60).collect::<String>()
                )));
            }
            statements.push(line.to_string());
        }
    }
    if !complete {
        return Err(AppError::bad(
            "Backup file is truncated (missing end marker)",
        ));
    }
    Ok(BackupInfo {
        schema_version: schema_version
            .ok_or_else(|| AppError::bad("Backup is missing its schema version"))?,
        counts: counts.ok_or_else(|| AppError::bad("Backup is missing its row counts"))?,
        statements,
    })
}

/// Restores a backup into an **empty** database (migrations are applied first).
pub async fn import(db: &Db, text: &str) -> AppResult<Counts> {
    let info = parse(text)?;
    migrate::run(db).await?;
    let conn = db.conn().await?;
    let version = migrate::applied(&conn)
        .await?
        .last()
        .map(|(v, _)| *v)
        .unwrap_or(0);
    if info.schema_version > version {
        return Err(AppError::bad(format!(
            "Backup schema version {} is newer than this build ({version}); upgrade Hourwise first",
            info.schema_version
        )));
    }
    let before = table_counts(&conn).await?;
    if before.values().any(|&n| n > 0) {
        return Err(AppError::bad(format!(
            "Refusing to restore into a database that already has data ({}). \
             Restore into a new, empty database instead.",
            format_counts(&before)
        )));
    }

    let tx = db::write_tx(&conn).await?;
    for chunk in info.statements.chunks(BATCH) {
        tx.execute_batch(&chunk.join("\n")).await?;
    }
    let after = table_counts(&tx).await?;
    for table in TABLES {
        let expected = info.counts.get(*table).copied().unwrap_or(0);
        let actual = after.get(*table).copied().unwrap_or(0);
        if expected != actual {
            // Dropping `tx` without committing rolls everything back.
            return Err(AppError::Internal(format!(
                "row count mismatch for {table}: backup says {expected}, restored {actual}"
            )));
        }
    }
    tx.commit().await?;
    Ok(after)
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

/// Data-integrity checks that work the same locally and on Turso Cloud.
pub async fn verify(db: &Db) -> AppResult<Report> {
    let conn = db.conn().await?;
    let mut report = Report {
        counts: table_counts(&conn).await?,
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
             GROUP BY user_id, entry_date HAVING SUM(minutes) > 1440)",
        ),
        (
            "invalid dates",
            "SELECT COUNT(*) FROM time_entries WHERE date(entry_date) IS NOT entry_date",
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
        let n = db::query_i64(&conn, sql, args![]).await?;
        if n > 0 {
            report.problems.push(format!("{n} {label}"));
        }
    }
    let admins = db::query_i64(
        &conn,
        "SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = 1",
        args![],
    )
    .await?;
    if admins == 0 {
        report.problems.push("no active admin account".into());
    }

    match db::query_value(&conn, "PRAGMA integrity_check", args![]).await {
        Ok(Some(Value::Text(s))) if s == "ok" => report.notes.push("integrity_check: ok".into()),
        Ok(other) => report
            .problems
            .push(format!("integrity_check reported: {other:?}")),
        Err(e) => report.notes.push(format!(
            "integrity_check not available on this connection ({})",
            e.detail()
        )),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_are_single_line_and_escaped() {
        assert_eq!(sql_literal(&Value::Null), "NULL");
        assert_eq!(sql_literal(&Value::Integer(-3)), "-3");
        assert_eq!(sql_literal(&Value::Text("O'Brien".into())), "'O''Brien'");
        let multi = sql_literal(&Value::Text("a\nb".into()));
        assert_eq!(multi, "CAST(X'610a62' AS TEXT)");
        assert!(!multi.contains('\n'));
    }

    #[test]
    fn parse_rejects_foreign_statements_and_truncation() {
        let good = format!(
            "{HEADER}\n-- schema_version: 1\n-- counts: users=1\n\
             INSERT INTO users (id) VALUES ('x');\n-- end of backup\n"
        );
        let info = parse(&good);
        assert!(info.is_ok());
        assert_eq!(info.map(|i| i.statements.len()).ok(), Some(1));

        let evil = good.replace("INSERT INTO users (id) VALUES ('x');", "DROP TABLE users;");
        assert!(parse(&evil).is_err());
        let other_table = good.replace("INSERT INTO users", "INSERT INTO _hourwise_migrations");
        assert!(parse(&other_table).is_err());
        let smuggled = good.replace(
            "VALUES ('x');",
            "VALUES ('x'); DROP TABLE users; INSERT INTO users (id) VALUES ('y');",
        );
        assert!(parse(&smuggled).is_err());
        assert!(single_statement("INSERT INTO t (a) VALUES ('a;b''c');"));
        let truncated = good.replace("-- end of backup\n", "");
        assert!(parse(&truncated).is_err());
        assert!(parse("hello").is_err());
    }
}
