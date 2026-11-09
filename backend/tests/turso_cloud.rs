//! Real Turso Cloud connectivity test. **Ignored by default**: it needs network access and a
//! dedicated, EMPTY test database. Never point it at the database holding employee records.
//!
//! Put the test database's URL and token in the root `.env` and run `npm run test:turso`:
//!
//! ```bash
//! turso db create hourwise-test
//! turso db show hourwise-test --url      # -> HOURWISE_TEST_TURSO_URL
//! turso db tokens create hourwise-test   # -> HOURWISE_TEST_TURSO_TOKEN
//! ```
//!
//! Safeguards, all checked before anything is written:
//! - only `HOURWISE_TEST_TURSO_URL` / `HOURWISE_TEST_TURSO_TOKEN` are read; there is no fallback
//!   to `TURSO_DATABASE_URL`;
//! - it refuses to run if the test URL is the production URL (`TURSO_DATABASE_URL`, or
//!   `HOURWISE_PROD_TURSO_URL`, which `scripts/cargo.mjs` forwards into Docker without the token);
//! - it refuses to run unless every Hourwise table is absent or empty. This check happens
//!   **before** migrations, so a wrong database is never altered.
//!
//! When it finishes, pass or fail, it deletes the rows it created. The schema is left in place.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use hourwise_backend::{
    args, backup,
    db::{self, Db, DbTarget},
    migrate,
};
use serde_json::json;

const DATA_TABLES: [&str; 4] = ["time_entries", "project_members", "projects", "users"];

/// `libsql://host`, `https://host/` and `HTTPS://HOST` all name the same database.
fn normalise(url: &str) -> String {
    let url = url.trim().trim_end_matches('/').to_ascii_lowercase();
    let host = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    host.to_string()
}

fn target() -> DbTarget {
    let url = std::env::var("HOURWISE_TEST_TURSO_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .expect("set HOURWISE_TEST_TURSO_URL to a dedicated, empty Turso test database");
    for var in ["TURSO_DATABASE_URL", "HOURWISE_PROD_TURSO_URL"] {
        if let Ok(prod) = std::env::var(var) {
            assert!(
                prod.trim().is_empty() || normalise(&prod) != normalise(&url),
                "refusing to run: HOURWISE_TEST_TURSO_URL is the production database ({var})"
            );
        }
    }
    let token = std::env::var("HOURWISE_TEST_TURSO_TOKEN").ok();
    let target = DbTarget::parse(&url, token).unwrap();
    assert!(
        target.is_remote(),
        "HOURWISE_TEST_TURSO_URL must be a libsql:// URL"
    );
    target
}

/// Refuses to continue unless every Hourwise data table is missing (fresh database) or empty.
/// It only reads, so it is safe on any database.
async fn assert_empty(db: &Db) {
    let conn = db.conn().await.unwrap();
    for table in DATA_TABLES {
        let exists = db::query_i64(
            &conn,
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            args![table],
        )
        .await
        .unwrap();
        if exists == 0 {
            continue;
        }
        // `table` comes from the constant list above, never from input.
        let rows = db::query_i64(&conn, &format!("SELECT COUNT(*) FROM {table}"), args![])
            .await
            .unwrap();
        assert_eq!(
            rows, 0,
            "refusing to run: table `{table}` in the Turso test database is not empty \
             (is this a real database?). Use a dedicated, empty test database."
        );
    }
}

async fn wipe(db: &Db) {
    let conn = db.conn().await.unwrap();
    for table in DATA_TABLES {
        conn.execute(&format!("DELETE FROM {table}"), ())
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "needs HOURWISE_TEST_TURSO_URL / HOURWISE_TEST_TURSO_TOKEN and network access"]
async fn turso_cloud_end_to_end() {
    let target = target();
    let db = Db::open(&target).await.unwrap();
    db.ping()
        .await
        .expect("cannot reach Turso: check the URL and token");
    // Read-only safety check BEFORE anything (including migrations) writes to the database.
    assert_empty(&db).await;
    migrate::run(&db).await.unwrap();

    let result = tokio::spawn(run_flow(target.clone())).await;
    wipe(&db).await;
    result.unwrap();
}

async fn run_flow(target: DbTarget) {
    // Full application on top of Turso Cloud.
    let app = TestApp::with_target(target).await;
    let admin = app.admin().await;
    let (user, project, s) = app.member_on_project(&admin, "cloud@test.local").await;

    // Writes, transactions and the daily cap over the network.
    let e = app
        .log_time(&s, &project, today(), 600, "cloud entry")
        .await;
    assert_eq!(e.status, StatusCode::CREATED, "{}", e.body);
    assert_eq!(
        app.log_time(&s, &project, today(), 841, "").await.status,
        StatusCode::BAD_REQUEST
    );
    let edited = app
        .patch(
            &format!("/api/time-entries/{}", e.id()),
            // PATCH is a full update (like the UI's EntryModal): send every field, or an
            // omitted description is cleared.
            json!({
                "project_id": project,
                "entry_date": today().to_string(),
                "minutes": 840,
                "description": "cloud entry"
            }),
            &s,
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["minutes"], 840);
    assert_eq!(edited.body["description"], "cloud entry");

    // Concurrent writers can't overshoot the cap on Turso either.
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let router = app.router.clone();
        let req = build(
            Method::POST,
            "/api/time-entries",
            Some(
                json!({ "project_id": project, "entry_date": today().to_string(), "minutes": 200 }),
            ),
            Some(&s),
            true,
        );
        tasks.push(tokio::spawn(
            async move { send_to(&router, req).await.status },
        ));
    }
    let mut created = 0;
    for t in tasks {
        if t.await.unwrap() == StatusCode::CREATED {
            created += 1;
        }
    }
    assert_eq!(created, 3, "600 remaining minutes / 200 = 3");

    // Constraints.
    let dup = app
        .post(
            "/api/admin/users",
            json!({ "email": "cloud@test.local", "name": "Dup", "password": "duppass123" }),
            &admin,
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);

    // Reports.
    let list = app
        .get(&format!("/api/admin/time-entries?user_id={user}"), &admin)
        .await;
    assert_eq!(total_minutes(&list.body), 1440);

    // Removing a member keeps history; deleting the project cascades explicitly.
    app.delete(
        &format!("/api/admin/projects/{project}/members/{user}"),
        &admin,
    )
    .await;
    let detail = app
        .get(&format!("/api/admin/projects/{project}"), &admin)
        .await;
    assert_eq!(detail.body["project"]["total_minutes"], 1440);

    // Backup export works against the cloud database and the data is consistent.
    let dump = backup::export(&app.state.db).await.unwrap();
    assert!(dump.contains("cloud entry"));
    assert!(backup::verify(&app.state.db).await.unwrap().ok());

    app.delete(&format!("/api/admin/projects/{project}"), &admin)
        .await;
    let conn = app.state.db.conn().await.unwrap();
    assert_eq!(
        db::query_i64(&conn, "SELECT COUNT(*) FROM time_entries", args![])
            .await
            .unwrap(),
        0
    );

    // Persistence: a fresh application instance sees the same data.
    let again = app.restart().await;
    again.login("cloud@test.local", "memberpass1").await;
}
