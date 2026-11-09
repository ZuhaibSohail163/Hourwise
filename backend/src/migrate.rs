//! Embedded schema migrations.
//!
//! Each file in `migrations/` is compiled into the binary and applied once, in order, inside a
//! transaction. Applied versions are recorded in `_hourwise_migrations` together with a checksum,
//! so editing a migration that has already run is detected and refused.

use libsql::Connection;

use crate::{
    args,
    db::{self, Db},
    error::{AppError, AppResult},
};

pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Every migration, in order. Add new files here; never edit or reorder applied ones.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "session_epoch",
        sql: include_str!("../migrations/0002_session_epoch.sql"),
    },
];

/// FNV-1a over the normalized SQL (line endings ignored so a CRLF checkout doesn't matter).
pub fn checksum(sql: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in sql.bytes().filter(|&b| b != b'\r') {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

async fn ensure_table(conn: &Connection) -> AppResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _hourwise_migrations (
            version    INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            checksum   TEXT NOT NULL,
            applied_at TEXT NOT NULL
        )",
        (),
    )
    .await?;
    Ok(())
}

/// Returns `(version, checksum)` for every applied migration.
pub async fn applied(conn: &Connection) -> AppResult<Vec<(i64, String)>> {
    ensure_table(conn).await?;
    let rows = db::query_records(
        conn,
        "SELECT version, checksum FROM _hourwise_migrations ORDER BY version",
        args![],
    )
    .await?;
    rows.iter()
        .map(|r| Ok((r.i64("version")?, r.text("checksum")?)))
        .collect()
}

/// Applies pending migrations. Returns how many were applied.
pub async fn run(db: &Db) -> AppResult<usize> {
    let conn = db.conn().await?;
    let done = applied(&conn).await?;

    for (version, sum) in &done {
        match MIGRATIONS.iter().find(|m| m.version == *version) {
            Some(m) if &checksum(m.sql) != sum => {
                return Err(AppError::Internal(format!(
                    "migration {version} ({}) was modified after it was applied; \
                     add a new migration instead of editing an old one",
                    m.name
                )));
            }
            Some(_) => {}
            None => {
                return Err(AppError::Internal(format!(
                    "database has migration {version}, which this build does not know about; \
                     is an older version of Hourwise running against a newer database?"
                )));
            }
        }
    }

    let mut count = 0;
    for m in MIGRATIONS {
        if done.iter().any(|(v, _)| *v == m.version) {
            continue;
        }
        let tx = db::write_tx(&conn).await?;
        tx.execute_batch(m.sql).await.map_err(|e| {
            AppError::Internal(format!("migration {} ({}) failed: {e}", m.version, m.name))
        })?;
        tx.execute(
            "INSERT INTO _hourwise_migrations (version, name, checksum, applied_at)
             VALUES (?1, ?2, ?3, ?4)",
            args![m.version, m.name, checksum(m.sql), db::now_ts()],
        )
        .await?;
        tx.commit().await?;
        tracing::info!("applied migration {:04}_{}", m.version, m.name);
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_unique_and_ordered() {
        for pair in MIGRATIONS.windows(2) {
            assert!(pair[0].version < pair[1].version);
        }
        assert_eq!(MIGRATIONS.first().map(|m| m.version), Some(1));
    }

    #[test]
    fn checksum_ignores_line_endings() {
        assert_eq!(checksum("a\r\nb"), checksum("a\nb"));
        assert_ne!(checksum("a"), checksum("b"));
    }
}
