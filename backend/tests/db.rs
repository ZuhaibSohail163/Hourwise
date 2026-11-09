//! Database integration tests: schema, migrations, constraints, transactions, concurrency,
//! persistence, backups and failure handling. Each test uses its own temporary database file.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use hourwise_backend::{
    args, backup,
    db::{self, Db, DbTarget},
    error::AppError,
    migrate,
};
use tempfile::TempDir;
use uuid::Uuid;

async fn fresh() -> (TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let target = DbTarget::Local {
        path: dir.path().join("t.db").to_string_lossy().into_owned(),
    };
    let db = Db::open(&target).await.unwrap();
    migrate::run(&db).await.unwrap();
    (dir, db)
}

async fn insert_user(db: &Db, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    let conn = db.conn().await.unwrap();
    db::execute(
        &conn,
        "INSERT INTO users (id, email, name, password_hash) VALUES (?1, ?2, 'U', 'h')",
        args![id, email],
    )
    .await
    .unwrap();
    id
}

async fn insert_project(db: &Db) -> Uuid {
    let id = Uuid::new_v4();
    let conn = db.conn().await.unwrap();
    db::execute(
        &conn,
        "INSERT INTO projects (id, name) VALUES (?1, 'P')",
        args![id],
    )
    .await
    .unwrap();
    id
}

async fn count(db: &Db, table: &str) -> i64 {
    let conn = db.conn().await.unwrap();
    db::query_i64(&conn, &format!("SELECT COUNT(*) FROM {table}"), args![])
        .await
        .unwrap()
}

#[tokio::test]
async fn connects_and_pings_local_database() {
    let (_dir, db) = fresh().await;
    db.ping().await.unwrap();
    assert!(!db.is_remote());
}

#[tokio::test]
async fn migrations_apply_once_and_create_schema() {
    let (_dir, db) = fresh().await;
    let conn = db.conn().await.unwrap();
    let applied = migrate::applied(&conn).await.unwrap();
    assert_eq!(applied.len(), migrate::MIGRATIONS.len());
    // Running again is a no-op.
    assert_eq!(migrate::run(&db).await.unwrap(), 0);

    for table in backup::TABLES {
        let n = db::query_i64(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            args![*table],
        )
        .await
        .unwrap();
        assert_eq!(n, 1, "table {table} missing");
    }
    for index in [
        "project_members_user_idx",
        "time_entries_user_date_idx",
        "time_entries_project_date_idx",
    ] {
        let n = db::query_i64(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            args![index],
        )
        .await
        .unwrap();
        assert_eq!(n, 1, "index {index} missing");
    }
    // Migration 0002 added the session epoch column with default 0.
    let user = insert_user(&db, "e@x.io").await;
    let epoch = db::query_i64(
        &conn,
        "SELECT session_epoch FROM users WHERE id = ?1",
        args![user],
    )
    .await
    .unwrap();
    assert_eq!(epoch, 0);
}

#[tokio::test]
async fn edited_migration_is_detected() {
    let (_dir, db) = fresh().await;
    let conn = db.conn().await.unwrap();
    db::execute(
        &conn,
        "UPDATE _hourwise_migrations SET checksum = 'tampered' WHERE version = 1",
        args![],
    )
    .await
    .unwrap();
    let err = migrate::run(&db).await.unwrap_err();
    assert!(err.detail().contains("modified"), "{}", err.detail());
}

#[tokio::test]
async fn unknown_newer_migration_is_refused() {
    let (_dir, db) = fresh().await;
    let conn = db.conn().await.unwrap();
    db::execute(
        &conn,
        "INSERT INTO _hourwise_migrations VALUES (999, 'future', 'x', 'now')",
        args![],
    )
    .await
    .unwrap();
    assert!(migrate::run(&db).await.is_err());
}

#[tokio::test]
async fn user_crud_and_unique_email() {
    let (_dir, db) = fresh().await;
    let id = insert_user(&db, "a@x.io").await;
    let conn = db.conn().await.unwrap();

    let dup = db::execute(
        &conn,
        "INSERT INTO users (id, email, name, password_hash) VALUES (?1, 'a@x.io', 'B', 'h')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap_err();
    assert!(matches!(dup, AppError::Conflict(_)), "{dup:?}");

    // Upper-case emails are rejected by the schema (the app lower-cases them first).
    let upper = db::execute(
        &conn,
        "INSERT INTO users (id, email, name, password_hash) VALUES (?1, 'B@X.IO', 'B', 'h')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap_err();
    assert!(matches!(upper, AppError::BadRequest(_)), "{upper:?}");

    let n = db::execute(
        &conn,
        "UPDATE users SET name = 'Renamed', is_active = 0 WHERE id = ?1",
        args![id],
    )
    .await
    .unwrap();
    assert_eq!(n, 1);
    let rec = db::query_records(&conn, "SELECT * FROM users WHERE id = ?1", args![id])
        .await
        .unwrap();
    assert_eq!(rec[0].text("name").unwrap(), "Renamed");
    assert!(!rec[0].bool("is_active").unwrap());
    assert_eq!(rec[0].uuid("id").unwrap(), id);
    // Default timestamps are RFC 3339 UTC.
    assert!(rec[0].ts("created_at").is_ok());

    assert_eq!(
        db::execute(&conn, "DELETE FROM users WHERE id = ?1", args![id])
            .await
            .unwrap(),
        1
    );
    assert_eq!(count(&db, "users").await, 0);
}

#[tokio::test]
async fn check_constraints_and_strict_types() {
    let (_dir, db) = fresh().await;
    let user = insert_user(&db, "c@x.io").await;
    let project = insert_project(&db).await;
    let conn = db.conn().await.unwrap();
    let insert_entry = |minutes: libsql::Value, date: &'static str| {
        let conn = conn.clone();
        async move {
            db::execute(
                &conn,
                "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                args![Uuid::new_v4(), user, project, date, minutes],
            )
            .await
        }
    };
    assert!(insert_entry(libsql::Value::Integer(60), "2026-01-05")
        .await
        .is_ok());
    for bad in [0, -1, 1441] {
        let e = insert_entry(libsql::Value::Integer(bad), "2026-01-05").await;
        assert!(matches!(e, Err(AppError::BadRequest(_))), "{bad}: {e:?}");
    }
    // STRICT: text in an INTEGER column is rejected.
    assert!(
        insert_entry(libsql::Value::Text("abc".into()), "2026-01-05")
            .await
            .is_err()
    );
    // Date format is enforced.
    assert!(insert_entry(libsql::Value::Integer(60), "2026/01/05")
        .await
        .is_err());
    assert!(insert_entry(libsql::Value::Integer(60), "05-01-2026")
        .await
        .is_err());

    for sql in [
        "INSERT INTO users (id, email, name, password_hash, role) VALUES (?1, 'r@x.io', 'R', 'h', 'owner')",
        "INSERT INTO users (id, email, name, password_hash, is_active) VALUES (?1, 'b@x.io', 'B', 'h', 2)",
        "INSERT INTO projects (id, name, is_archived) VALUES (?1, 'P', 5)",
    ] {
        assert!(
            db::execute(&conn, sql, args![Uuid::new_v4()]).await.is_err(),
            "{sql}"
        );
    }
    // IDs must look like UUIDs.
    assert!(db::execute(
        &conn,
        "INSERT INTO projects (id, name) VALUES ('short', 'P')",
        args![]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn foreign_keys_and_cascades_on_local_connections() {
    let (_dir, db) = fresh().await;
    let user = insert_user(&db, "f@x.io").await;
    let project = insert_project(&db).await;
    let conn = db.conn().await.unwrap();

    let missing = db::execute(
        &conn,
        "INSERT INTO project_members (project_id, user_id) VALUES (?1, ?2)",
        args![project, Uuid::new_v4()],
    )
    .await
    .unwrap_err();
    assert!(matches!(missing, AppError::BadRequest(_)), "{missing:?}");

    db::execute(
        &conn,
        "INSERT INTO project_members (project_id, user_id) VALUES (?1, ?2)",
        args![project, user],
    )
    .await
    .unwrap();
    // Duplicate membership violates the composite primary key.
    let dup = db::execute(
        &conn,
        "INSERT INTO project_members (project_id, user_id) VALUES (?1, ?2)",
        args![project, user],
    )
    .await
    .unwrap_err();
    assert!(matches!(dup, AppError::Conflict(_)), "{dup:?}");

    db::execute(
        &conn,
        "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
         VALUES (?1, ?2, ?3, '2026-01-05', 30)",
        args![Uuid::new_v4(), user, project],
    )
    .await
    .unwrap();
    db::execute(&conn, "DELETE FROM projects WHERE id = ?1", args![project])
        .await
        .unwrap();
    assert_eq!(count(&db, "project_members").await, 0);
    assert_eq!(count(&db, "time_entries").await, 0);
    assert_eq!(count(&db, "users").await, 1);
}

#[tokio::test]
async fn transactions_commit_and_roll_back() {
    let (_dir, db) = fresh().await;
    let conn = db.conn().await.unwrap();

    {
        let tx = db::write_tx(&conn).await.unwrap();
        db::execute(
            &tx,
            "INSERT INTO projects (id, name) VALUES (?1, 'rolled back')",
            args![Uuid::new_v4()],
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
    }
    assert_eq!(count(&db, "projects").await, 0);

    {
        // Dropping an uncommitted transaction also rolls back.
        let tx = db::write_tx(&conn).await.unwrap();
        db::execute(
            &tx,
            "INSERT INTO projects (id, name) VALUES (?1, 'dropped')",
            args![Uuid::new_v4()],
        )
        .await
        .unwrap();
        drop(tx);
    }
    assert_eq!(count(&db, "projects").await, 0);

    let tx = db::write_tx(&conn).await.unwrap();
    db::execute(
        &tx,
        "INSERT INTO projects (id, name) VALUES (?1, 'kept')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(count(&db, "projects").await, 1);
}

#[tokio::test]
async fn failed_statement_inside_transaction_leaves_no_partial_writes() {
    let (_dir, db) = fresh().await;
    let conn = db.conn().await.unwrap();
    let result = async {
        let tx = db::write_tx(&conn).await?;
        db::execute(
            &tx,
            "INSERT INTO users (id, email, name, password_hash) VALUES (?1, 'p@x.io', 'P', 'h')",
            args![Uuid::new_v4()],
        )
        .await?;
        // Violates the unique email constraint.
        db::execute(
            &tx,
            "INSERT INTO users (id, email, name, password_hash) VALUES (?1, 'p@x.io', 'P', 'h')",
            args![Uuid::new_v4()],
        )
        .await?;
        tx.commit().await?;
        Ok::<_, AppError>(())
    }
    .await;
    assert!(result.is_err());
    assert_eq!(count(&db, "users").await, 0);
}

#[tokio::test]
async fn concurrent_writers_all_succeed() {
    let (_dir, db) = fresh().await;
    let mut tasks = Vec::new();
    for i in 0..25 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            let conn = db.conn().await?;
            let tx = db::write_tx(&conn).await?;
            db::execute(
                &tx,
                "INSERT INTO users (id, email, name, password_hash) VALUES (?1, ?2, 'C', 'h')",
                args![Uuid::new_v4(), format!("c{i}@x.io")],
            )
            .await?;
            tx.commit().await?;
            Ok::<_, AppError>(())
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    assert_eq!(count(&db, "users").await, 25);

    // Concurrent readers while writing.
    let readers: Vec<_> = (0..10)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move { count(&db, "users").await })
        })
        .collect();
    for r in readers {
        assert_eq!(r.await.unwrap(), 25);
    }
}

#[tokio::test]
async fn data_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let target = DbTarget::Local {
        path: dir.path().join("persist.db").to_string_lossy().into_owned(),
    };
    {
        let db = Db::open(&target).await.unwrap();
        migrate::run(&db).await.unwrap();
        insert_user(&db, "persist@x.io").await;
    }
    let db = Db::open(&target).await.unwrap();
    assert_eq!(migrate::run(&db).await.unwrap(), 0);
    assert_eq!(count(&db, "users").await, 1);
}

#[tokio::test]
async fn unreachable_remote_database_reports_unavailable() {
    // Nothing listens on port 9; the connection is refused immediately.
    let target = DbTarget::parse("http://127.0.0.1:9", None).unwrap();
    let started = Instant::now();
    let result = db::open_with_retry(&target, 1).await;
    let err = result.err().expect("connecting to a closed port must fail");
    assert!(
        matches!(err, AppError::Unavailable(_) | AppError::Internal(_)),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[tokio::test]
async fn backup_roundtrip_preserves_everything() {
    let (_dir, db) = fresh().await;
    let user = insert_user(&db, "o'brien@x.io").await;
    let project = insert_project(&db).await;
    let conn = db.conn().await.unwrap();
    db::execute(
        &conn,
        "INSERT INTO project_members (project_id, user_id) VALUES (?1, ?2)",
        args![project, user],
    )
    .await
    .unwrap();
    db::execute(
        &conn,
        "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes, description)
         VALUES (?1, ?2, ?3, '2024-02-29', 1440, ?4)",
        args![
            Uuid::new_v4(),
            user,
            project,
            "multi\nline 'quoted' \u{1f600}; DROP TABLE users;"
        ],
    )
    .await
    .unwrap();

    let dump = backup::export(&db).await.unwrap();
    let (_dir2, restored) = {
        let dir = tempfile::tempdir().unwrap();
        let target = DbTarget::Local {
            path: dir.path().join("r.db").to_string_lossy().into_owned(),
        };
        (dir, Db::open(&target).await.unwrap())
    };
    let counts = backup::import(&restored, &dump).await.unwrap();
    assert_eq!(counts.get("time_entries"), Some(&1));
    assert_eq!(counts.get("users"), Some(&1));

    // Exporting the restored database gives the same data (only the created_at line differs).
    let strip = |s: &str| {
        s.lines()
            .filter(|l| !l.starts_with("-- created_at"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        strip(&dump),
        strip(&backup::export(&restored).await.unwrap())
    );

    // Restoring twice is refused (target is not empty).
    assert!(backup::import(&restored, &dump).await.is_err());
}

#[tokio::test]
async fn restore_rejects_mismatched_counts_without_writing() {
    let (_dir, db) = fresh().await;
    insert_user(&db, "x@x.io").await;
    let dump = backup::export(&db).await.unwrap();
    let forged = dump.replace("users=1", "users=2");

    let (_d2, target) = fresh().await;
    assert!(backup::import(&target, &forged).await.is_err());
    assert_eq!(count(&target, "users").await, 0);
}

#[tokio::test]
async fn verify_reports_orphans_and_invalid_totals() {
    let (_dir, db) = fresh().await;
    // No admin yet -> reported.
    let report = backup::verify(&db).await.unwrap();
    assert!(report
        .problems
        .iter()
        .any(|p| p.contains("no active admin")));

    let conn = db.conn().await.unwrap();
    db::execute(
        &conn,
        "INSERT INTO users (id, email, name, password_hash, role) VALUES (?1, 'adm@x.io', 'A', 'h', 'admin')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap();
    assert!(backup::verify(&db).await.unwrap().ok());

    // Simulate a Turso connection without foreign keys: orphans + over-24h day.
    let user = insert_user(&db, "o@x.io").await;
    let project = insert_project(&db).await;
    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .await
        .unwrap();
    for minutes in [1000, 1000] {
        db::execute(
            &conn,
            "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
             VALUES (?1, ?2, ?3, '2026-01-05', ?4)",
            args![Uuid::new_v4(), user, project, minutes],
        )
        .await
        .unwrap();
    }
    db::execute(
        &conn,
        "INSERT INTO project_members (project_id, user_id) VALUES (?1, ?2)",
        args![Uuid::new_v4(), user],
    )
    .await
    .unwrap();
    let report = backup::verify(&db).await.unwrap();
    assert!(!report.ok());
    assert!(
        report.problems.iter().any(|p| p.contains("memberships")),
        "{:?}",
        report.problems
    );
    assert!(
        report.problems.iter().any(|p| p.contains("24 hours")),
        "{:?}",
        report.problems
    );
}

#[tokio::test]
async fn record_type_mismatch_is_an_error_not_a_panic() {
    let (_dir, db) = fresh().await;
    let conn = db.conn().await.unwrap();
    let rows = db::query_records(&conn, "SELECT 'text' AS a, 5 AS b, NULL AS c", args![])
        .await
        .unwrap();
    let r = &rows[0];
    assert!(r.i64("a").is_err());
    assert!(r.text("b").is_err());
    assert!(r.text("c").is_err());
    assert_eq!(r.opt_text("c").unwrap(), None);
    assert!(r.text("missing").is_err());
    assert!(r.uuid("a").is_err());
}
