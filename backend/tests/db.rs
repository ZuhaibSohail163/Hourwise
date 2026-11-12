//! Database integration tests: connectivity, migrations, schema, constraints, transactions,
//! row locking, concurrency, pool limits, timeouts, persistence, integrity report and the test
//! runner's own safety checks. Each test uses its own freshly created PostgreSQL database on the
//! disposable test server (see `common/mod.rs`).
#![allow(clippy::unwrap_used)]

mod common;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use common::{check_not_protected, TestDatabase};
use hourwise_backend::{
    args,
    db::{self, destination, Db, DbSettings},
    error::AppError,
    migrate, verify,
};
use uuid::Uuid;

/// A fresh, migrated database.
async fn fresh() -> (Arc<TestDatabase>, Db) {
    let database = TestDatabase::create().await;
    let db = database.connect().await;
    migrate::run(&db).await.unwrap();
    (database, db)
}

async fn insert_user(db: &Db, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    let mut conn = db.conn().await.unwrap();
    db::execute(
        &mut conn,
        "INSERT INTO users (id, email, name, password_hash) VALUES ($1, $2, 'U', 'h')",
        args![id, email],
    )
    .await
    .unwrap();
    id
}

async fn insert_project(db: &Db) -> Uuid {
    let id = Uuid::new_v4();
    let mut conn = db.conn().await.unwrap();
    db::execute(
        &mut conn,
        "INSERT INTO projects (id, name) VALUES ($1, 'P')",
        args![id],
    )
    .await
    .unwrap();
    id
}

async fn count(db: &Db, table: &str) -> i64 {
    let mut conn = db.conn().await.unwrap();
    db::query_i64(&mut conn, &format!("SELECT COUNT(*) FROM {table}"), args![])
        .await
        .unwrap()
}

#[tokio::test]
async fn connects_and_pings() {
    let (database, db) = fresh().await;
    db.ping().await.unwrap();
    let description = database.settings.describe();
    assert!(description.contains(&database.name), "{description}");
}

#[tokio::test]
async fn migrations_apply_once_and_create_schema() {
    let (_database, db) = fresh().await;
    assert_eq!(migrate::applied(&db).await.unwrap().len(), migrate::known());
    // Running again is a no-op.
    assert_eq!(migrate::run(&db).await.unwrap(), 0);

    let mut conn = db.conn().await.unwrap();
    for table in verify::TABLES {
        let exists = db::query_scalar::<bool>(
            &mut conn,
            "SELECT to_regclass($1) IS NOT NULL",
            args![format!("public.{table}")],
        )
        .await
        .unwrap();
        assert_eq!(exists, Some(true), "table {table} missing");
    }
    for index in [
        "project_members_user_idx",
        "time_entries_user_date_idx",
        "time_entries_project_date_idx",
        "time_entries_date_idx",
    ] {
        let n = db::query_i64(
            &mut conn,
            "SELECT COUNT(*) FROM pg_indexes WHERE schemaname = 'public' AND indexname = $1",
            args![index],
        )
        .await
        .unwrap();
        assert_eq!(n, 1, "index {index} missing");
    }
    // Native PostgreSQL types (not text) for ids, booleans, dates and timestamps.
    for (table, column, expected) in [
        ("users", "id", "uuid"),
        ("users", "is_active", "boolean"),
        ("users", "session_epoch", "bigint"),
        ("users", "created_at", "timestamp with time zone"),
        ("time_entries", "entry_date", "date"),
        ("time_entries", "minutes", "integer"),
        ("projects", "is_archived", "boolean"),
    ] {
        let ty = db::query_scalar::<String>(
            &mut conn,
            "SELECT data_type FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2",
            args![table, column],
        )
        .await
        .unwrap();
        assert_eq!(ty.as_deref(), Some(expected), "{table}.{column}");
    }
    let user = insert_user(&db, "e@x.io").await;
    let epoch = db::query_i64(
        &mut conn,
        "SELECT session_epoch FROM users WHERE id = $1",
        args![user],
    )
    .await
    .unwrap();
    assert_eq!(epoch, 0);
}

#[tokio::test]
async fn concurrent_migration_runs_apply_each_migration_once() {
    // Several backend instances starting at the same time against an empty database.
    let database = TestDatabase::create().await;
    let db = database.connect().await;
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move { migrate::run(&db).await })
        })
        .collect();
    // Every instance succeeds (SQLx serialises them with an advisory lock)...
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    // ...and each migration is recorded exactly once.
    assert_eq!(migrate::applied(&db).await.unwrap().len(), migrate::known());
    let mut conn = db.conn().await.unwrap();
    let rows = db::query_i64(&mut conn, "SELECT COUNT(*) FROM _sqlx_migrations", args![])
        .await
        .unwrap();
    assert_eq!(rows, i64::try_from(migrate::known()).unwrap());
}

#[tokio::test]
async fn edited_migration_is_detected() {
    let (_database, db) = fresh().await;
    let mut conn = db.conn().await.unwrap();
    db::execute(
        &mut conn,
        "UPDATE _sqlx_migrations SET checksum = '\\x00'::bytea WHERE version = 1",
        args![],
    )
    .await
    .unwrap();
    let err = migrate::run(&db).await.unwrap_err();
    assert!(err.detail().contains("modified"), "{}", err.detail());
}

#[tokio::test]
async fn unknown_newer_migration_is_refused() {
    let (_database, db) = fresh().await;
    let mut conn = db.conn().await.unwrap();
    db::execute(
        &mut conn,
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (999, 'future', TRUE, '\\x00'::bytea, 0)",
        args![],
    )
    .await
    .unwrap();
    let err = migrate::run(&db).await.unwrap_err();
    assert!(err.detail().contains("999"), "{}", err.detail());
}

#[tokio::test]
async fn user_crud_and_unique_email() {
    let (_database, db) = fresh().await;
    let id = insert_user(&db, "a@x.io").await;
    let mut conn = db.conn().await.unwrap();

    let dup = db::execute(
        &mut conn,
        "INSERT INTO users (id, email, name, password_hash) VALUES ($1, 'a@x.io', 'B', 'h')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap_err();
    assert!(matches!(dup, AppError::Conflict(_)), "{dup:?}");

    // Upper-case and empty emails are rejected by the schema (the app lower-cases them first).
    for email in ["B@X.IO", ""] {
        let err = db::execute(
            &mut conn,
            "INSERT INTO users (id, email, name, password_hash) VALUES ($1, $2, 'B', 'h')",
            args![Uuid::new_v4(), email],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{email}: {err:?}");
    }

    let n = db::execute(
        &mut conn,
        "UPDATE users SET name = 'Renamed', is_active = FALSE WHERE id = $1",
        args![id],
    )
    .await
    .unwrap();
    assert_eq!(n, 1);
    let rec = db::query_records(&mut conn, "SELECT * FROM users WHERE id = $1", args![id])
        .await
        .unwrap();
    assert_eq!(rec[0].text("name").unwrap(), "Renamed");
    assert!(!rec[0].bool("is_active").unwrap());
    assert_eq!(rec[0].uuid("id").unwrap(), id);
    assert!(rec[0].ts("created_at").is_ok());

    assert_eq!(
        db::execute(&mut conn, "DELETE FROM users WHERE id = $1", args![id])
            .await
            .unwrap(),
        1
    );
    assert_eq!(count(&db, "users").await, 0);
}

#[tokio::test]
async fn check_constraints_and_types() {
    let (_database, db) = fresh().await;
    let user = insert_user(&db, "c@x.io").await;
    let project = insert_project(&db).await;
    let mut conn = db.conn().await.unwrap();

    let entry = "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
                 VALUES ($1, $2, $3, $4, $5)";
    db::execute(
        &mut conn,
        entry,
        args![
            Uuid::new_v4(),
            user,
            project,
            common::date("2026-01-05"),
            60
        ],
    )
    .await
    .unwrap();
    for bad in [0, -5, 1441] {
        let e = db::execute(
            &mut conn,
            entry,
            args![
                Uuid::new_v4(),
                user,
                project,
                common::date("2026-01-05"),
                bad
            ],
        )
        .await
        .unwrap_err();
        assert!(matches!(e, AppError::BadRequest(_)), "{bad}: {e:?}");
    }
    // Types are enforced: text is not an integer, and invalid calendar dates are rejected.
    assert!(db::execute(
        &mut conn,
        entry,
        args![
            Uuid::new_v4(),
            user,
            project,
            common::date("2026-01-05"),
            "abc"
        ]
    )
    .await
    .is_err());
    for bad_date in ["2026-02-30", "2026/01/05x", "05-01-2026x"] {
        assert!(
            db::execute(
                &mut conn,
                "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
                 VALUES ($1, $2, $3, $4::text::date, 60)",
                args![Uuid::new_v4(), user, project, bad_date],
            )
            .await
            .is_err(),
            "{bad_date}"
        );
    }
    for sql in [
        "INSERT INTO users (id, email, name, password_hash, role) VALUES ($1, 'r@x.io', 'R', 'h', 'owner')",
        "INSERT INTO users (id, email, name, password_hash) VALUES ($1, 'n@x.io', NULL, 'h')",
        "INSERT INTO projects (id, name) VALUES ($1, NULL)",
    ] {
        let e = db::execute(&mut conn, sql, args![Uuid::new_v4()])
            .await
            .unwrap_err();
        assert!(matches!(e, AppError::BadRequest(_)), "{sql}: {e:?}");
    }
    // IDs must be UUIDs.
    assert!(db::execute(
        &mut conn,
        "INSERT INTO projects (id, name) VALUES ('short', 'P')",
        args![]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn foreign_keys_and_cascades() {
    let (_database, db) = fresh().await;
    let user = insert_user(&db, "f@x.io").await;
    let project = insert_project(&db).await;
    let mut conn = db.conn().await.unwrap();

    let missing = db::execute(
        &mut conn,
        "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)",
        args![project, Uuid::new_v4()],
    )
    .await
    .unwrap_err();
    assert!(matches!(missing, AppError::BadRequest(_)), "{missing:?}");

    db::execute(
        &mut conn,
        "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)",
        args![project, user],
    )
    .await
    .unwrap();
    // Duplicate membership violates the composite primary key.
    let dup = db::execute(
        &mut conn,
        "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)",
        args![project, user],
    )
    .await
    .unwrap_err();
    assert!(matches!(dup, AppError::Conflict(_)), "{dup:?}");

    db::execute(
        &mut conn,
        "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
         VALUES ($1, $2, $3, DATE '2026-01-05', 30)",
        args![Uuid::new_v4(), user, project],
    )
    .await
    .unwrap();
    db::execute(
        &mut conn,
        "DELETE FROM projects WHERE id = $1",
        args![project],
    )
    .await
    .unwrap();
    assert_eq!(count(&db, "project_members").await, 0);
    assert_eq!(count(&db, "time_entries").await, 0);
    assert_eq!(count(&db, "users").await, 1);
}

#[tokio::test]
async fn transactions_commit_and_roll_back() {
    let (_database, db) = fresh().await;
    let mut conn = db.conn().await.unwrap();

    {
        let mut tx = db::write_tx(&mut conn).await.unwrap();
        db::execute(
            &mut tx,
            "INSERT INTO projects (id, name) VALUES ($1, 'rolled back')",
            args![Uuid::new_v4()],
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
    }
    assert_eq!(count(&db, "projects").await, 0);

    {
        // Dropping an uncommitted transaction also rolls back.
        let mut tx = db::write_tx(&mut conn).await.unwrap();
        db::execute(
            &mut tx,
            "INSERT INTO projects (id, name) VALUES ($1, 'dropped')",
            args![Uuid::new_v4()],
        )
        .await
        .unwrap();
        drop(tx);
    }
    assert_eq!(count(&db, "projects").await, 0);

    let mut tx = db::write_tx(&mut conn).await.unwrap();
    db::execute(
        &mut tx,
        "INSERT INTO projects (id, name) VALUES ($1, 'kept')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(count(&db, "projects").await, 1);
}

#[tokio::test]
async fn failed_statement_inside_transaction_leaves_no_partial_writes() {
    let (_database, db) = fresh().await;
    let mut conn = db.conn().await.unwrap();
    let result = async {
        let mut tx = db::write_tx(&mut conn).await?;
        db::execute(
            &mut tx,
            "INSERT INTO users (id, email, name, password_hash) VALUES ($1, 'p@x.io', 'P', 'h')",
            args![Uuid::new_v4()],
        )
        .await?;
        // Violates the unique email constraint.
        db::execute(
            &mut tx,
            "INSERT INTO users (id, email, name, password_hash) VALUES ($1, 'p@x.io', 'P', 'h')",
            args![Uuid::new_v4()],
        )
        .await?;
        tx.commit().await?;
        Ok::<_, AppError>(())
    }
    .await;
    assert!(matches!(result, Err(AppError::Conflict(_))), "{result:?}");
    assert_eq!(count(&db, "users").await, 0);
    // The pooled connection is still usable after the failed transaction.
    db.ping().await.unwrap();
}

#[tokio::test]
async fn user_row_lock_serialises_only_that_user() {
    let (_database, db) = fresh().await;
    let alice = insert_user(&db, "alice@x.io").await;
    let bob = insert_user(&db, "bob@x.io").await;

    let mut c1 = db.conn().await.unwrap();
    let mut tx1 = db::write_tx(&mut c1).await.unwrap();
    db::lock_user(&mut tx1, alice).await.unwrap();

    // Another transaction for Bob proceeds immediately.
    let db2 = db.clone();
    let bob_task = tokio::spawn(async move {
        let mut c = db2.conn().await.unwrap();
        let mut tx = db::write_tx(&mut c).await.unwrap();
        db::lock_user(&mut tx, bob).await.unwrap();
        tx.commit().await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), bob_task)
        .await
        .expect("locking another user must not block")
        .unwrap();

    // A second transaction for Alice waits until the first one finishes.
    let db3 = db.clone();
    let alice_task = tokio::spawn(async move {
        let mut c = db3.conn().await.unwrap();
        let mut tx = db::write_tx(&mut c).await.unwrap();
        db::lock_user(&mut tx, alice).await.unwrap();
        tx.commit().await.unwrap();
        Instant::now()
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !alice_task.is_finished(),
        "the second lock on alice must wait"
    );
    let released = Instant::now();
    tx1.commit().await.unwrap();
    let acquired = tokio::time::timeout(Duration::from_secs(5), alice_task)
        .await
        .expect("the lock must be granted after commit")
        .unwrap();
    assert!(acquired >= released);
}

#[tokio::test]
async fn concurrent_writers_all_succeed() {
    let (_database, db) = fresh().await;
    let mut tasks = Vec::new();
    for i in 0..25 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            let mut conn = db.conn().await?;
            let mut tx = db::write_tx(&mut conn).await?;
            db::execute(
                &mut tx,
                "INSERT INTO users (id, email, name, password_hash) VALUES ($1, $2, 'C', 'h')",
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
async fn data_persists_across_reconnect() {
    let (database, db) = fresh().await;
    insert_user(&db, "persist@x.io").await;
    db.close().await;

    let again = database.connect().await;
    assert_eq!(migrate::run(&again).await.unwrap(), 0);
    assert_eq!(count(&again, "users").await, 1);
}

#[tokio::test]
async fn unreachable_database_reports_unavailable() {
    // Nothing listens on port 9; the connection is refused immediately.
    let mut settings = DbSettings::new("postgres://nobody:pw@127.0.0.1:9/hourwise").unwrap();
    settings.acquire_timeout = Duration::from_secs(2);
    let started = Instant::now();
    let Err(err) = db::connect_with_retry(&settings, 1).await else {
        panic!("connecting to a closed port must fail");
    };
    assert!(matches!(err, AppError::Unavailable(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(30));
}

#[tokio::test]
async fn pool_exhaustion_returns_unavailable_instead_of_hanging() {
    let (database, _db) = fresh().await;
    let mut settings = database.settings.clone();
    settings.max_connections = 1;
    // Generous enough for opening the first connection while other tests load the machine.
    settings.acquire_timeout = Duration::from_secs(4);
    let db = Db::connect(&settings).await.unwrap();

    let _held = db.conn().await.unwrap();
    let started = Instant::now();
    let Err(err) = db.conn().await else {
        panic!("the only connection is in use");
    };
    assert!(matches!(err, AppError::Unavailable(_)), "{err:?}");
    // It gives up after the acquire timeout instead of waiting forever.
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(3), "{waited:?}");
    assert!(waited < Duration::from_secs(15), "{waited:?}");
}

#[tokio::test]
async fn statement_timeout_is_enforced() {
    let (database, _db) = fresh().await;
    let mut settings = database.settings.clone();
    settings.statement_timeout = Duration::from_secs(1);
    let db = Db::connect(&settings).await.unwrap();
    let mut conn = db.conn().await.unwrap();

    let started = Instant::now();
    let err = db::execute(&mut conn, "SELECT pg_sleep(5)", args![])
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Unavailable(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[tokio::test]
async fn verify_reports_orphans_and_invalid_totals() {
    let (_database, db) = fresh().await;
    // No admin yet -> reported.
    let report = verify::verify(&db).await.unwrap();
    assert!(report
        .problems
        .iter()
        .any(|p| p.contains("no active admin")));

    let mut conn = db.conn().await.unwrap();
    db::execute(
        &mut conn,
        "INSERT INTO users (id, email, name, password_hash, role) VALUES ($1, 'adm@x.io', 'A', 'h', 'admin')",
        args![Uuid::new_v4()],
    )
    .await
    .unwrap();
    let report = verify::verify(&db).await.unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert!(report.notes.iter().any(|n| n.starts_with("PostgreSQL")));

    // Bypass the foreign keys (superuser-only, test server) to simulate damaged data, and
    // exceed the 24h cap that only the application enforces.
    let user = insert_user(&db, "o@x.io").await;
    let project = insert_project(&db).await;
    for minutes in [1000, 1000] {
        db::execute(
            &mut conn,
            "INSERT INTO time_entries (id, user_id, project_id, entry_date, minutes)
             VALUES ($1, $2, $3, DATE '2026-01-05', $4)",
            args![Uuid::new_v4(), user, project, minutes],
        )
        .await
        .unwrap();
    }
    db::execute(&mut conn, "SET session_replication_role = replica", args![])
        .await
        .unwrap();
    db::execute(
        &mut conn,
        "INSERT INTO project_members (project_id, user_id) VALUES ($1, $2)",
        args![Uuid::new_v4(), user],
    )
    .await
    .unwrap();
    db::execute(&mut conn, "SET session_replication_role = DEFAULT", args![])
        .await
        .unwrap();
    let report = verify::verify(&db).await.unwrap();
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
    let (_database, db) = fresh().await;
    let mut conn = db.conn().await.unwrap();
    let rows = db::query_records(
        &mut conn,
        "SELECT 'text'::text AS a, 5 AS b, NULL::text AS c",
        args![],
    )
    .await
    .unwrap();
    let r = &rows[0];
    assert!(r.i64("a").is_err());
    assert!(r.text("b").is_err());
    assert!(r.text("c").is_err());
    assert_eq!(r.opt_text("c").unwrap(), None);
    assert!(r.text("missing").is_err());
    assert!(r.uuid("a").is_err());
    assert_eq!(r.i64("b").unwrap(), 5);
}

/// The test runner's own guard: a protected (production / development) database server must be
/// recognised however its URL is written.
#[test]
fn test_runner_refuses_protected_servers() {
    let protected =
        [destination("postgres://hourwise:prod-secret@db.example.com:5432/hourwise").unwrap()];
    for url in [
        "postgres://other:x@db.example.com/hourwise",
        "postgresql://u@DB.EXAMPLE.COM:5432/postgres",
        "postgres://u@db.example.com./scratch?sslmode=require",
    ] {
        let err = check_not_protected(&destination(url).unwrap(), &protected).unwrap_err();
        assert!(!err.contains("prod-secret"), "{err}");
    }
    let local = [destination("postgres://hourwise@localhost:5432/hourwise").unwrap()];
    for url in [
        "postgres://u@127.0.0.1/x",
        "postgres://u@[::1]:5432/x",
        "postgres://u@host.docker.internal:5432/x",
    ] {
        assert!(
            check_not_protected(&destination(url).unwrap(), &local).is_err(),
            "{url}"
        );
    }
    // The disposable server runs on another port.
    assert!(check_not_protected(
        &destination("postgres://t@localhost:55432/postgres").unwrap(),
        &local
    )
    .is_ok());
}
