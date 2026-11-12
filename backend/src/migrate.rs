//! Embedded schema migrations (SQLx).
//!
//! Every file in `migrations/` is compiled into the binary (`sqlx::migrate!`). On startup, or with
//! `hourwise-backend migrate`, pending migrations are applied in order, each in its own
//! transaction. SQLx records the version and a checksum in `_sqlx_migrations` and takes a
//! PostgreSQL advisory lock while migrating, so several backend instances starting at once are
//! safe. A migration that was edited after being applied, or an applied version this build doesn't
//! know, makes `run` fail instead of guessing.

use sqlx::migrate::{MigrateError, Migrator};

use crate::{
    args,
    db::{self, Db},
    error::{AppError, AppResult},
};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Number of migrations embedded in this build.
pub fn known() -> usize {
    MIGRATOR.iter().count()
}

fn migrate_error(e: MigrateError) -> AppError {
    match e {
        MigrateError::VersionMismatch(v) => AppError::Internal(format!(
            "migration {v} was modified after it was applied; never edit an applied migration, \
             add a new one instead"
        )),
        MigrateError::VersionMissing(v) => AppError::Internal(format!(
            "the database has migration {v} applied, which this build doesn't know \
             (is an older version of the backend running against a newer database?)"
        )),
        MigrateError::Execute(e) => AppError::from(e),
        other => AppError::Internal(format!("migration failed: {other}")),
    }
}

/// Versions of successfully applied migrations, oldest first. Empty for a fresh database.
pub async fn applied(db: &Db) -> AppResult<Vec<i64>> {
    let mut conn = db.conn().await?;
    let exists = db::query_scalar::<bool>(
        &mut conn,
        "SELECT to_regclass('_sqlx_migrations') IS NOT NULL",
        args![],
    )
    .await?
    .unwrap_or(false);
    if !exists {
        return Ok(Vec::new());
    }
    let rows = db::query_records(
        &mut conn,
        "SELECT version FROM _sqlx_migrations WHERE success ORDER BY version",
        args![],
    )
    .await?;
    rows.iter().map(|r| r.i64("version")).collect()
}

/// Applies pending migrations. Returns how many were applied by this call (if several instances
/// migrate at the same moment, the count can include ones another instance applied).
pub async fn run(db: &Db) -> AppResult<usize> {
    let before = applied(db).await?.len();
    MIGRATOR.run(db.pool()).await.map_err(migrate_error)?;
    let after = applied(db).await?.len();
    let n = after.saturating_sub(before);
    if n > 0 {
        tracing::info!("applied {n} database migration(s)");
    }
    Ok(n)
}
