//! API integration tests: every endpoint through the real router and a real (temporary)
//! database, checking both the HTTP responses and the resulting database state.
#![allow(clippy::unwrap_used)]

mod common;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use std::sync::Arc;

use chrono::{Datelike, Duration, NaiveDate};
use common::*;
use hourwise_backend::{args, db};
use serde_json::json;

#[tokio::test]
async fn health_endpoint() {
    let app = TestApp::new().await;
    let res = app
        .send(Request::get("/api/health").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn admin_is_seeded_once_and_can_log_in() {
    let app = TestApp::new().await;
    let res = app.try_login(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["user"]["role"], "admin");
    // Secrets never leave the API.
    assert!(res.body["user"].get("password_hash").is_none());
    assert!(res.body["user"].get("session_epoch").is_none());
    assert!(res.body.get("token").is_none());

    // Email is case-insensitive.
    let upper = app
        .try_login(&ADMIN_EMAIL.to_uppercase(), ADMIN_PASSWORD)
        .await;
    assert_eq!(upper.status, StatusCode::OK);

    // Restarting does not create a second admin.
    let again = app.restart().await;
    let s = again.admin().await;
    let users = again.get("/api/admin/users", &s).await;
    assert_eq!(users.body.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn login_failures() {
    let app = TestApp::new().await;
    assert_eq!(
        app.try_login(ADMIN_EMAIL, "wrong-password").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.try_login("nobody@test.local", ADMIN_PASSWORD)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(app.try_login("", "").await.status, StatusCode::UNAUTHORIZED);
    // Malformed JSON body.
    let res = app
        .send(
            Request::post("/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert!(res.status.is_client_error());
}

#[tokio::test]
async fn member_login_and_profile() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    app.create_user(&admin, "Ann@Example.com", "annpass123", "member")
        .await;
    let ann = app.login("ann@example.com", "annpass123").await;

    let me = app.get("/api/me", &ann).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["email"], "ann@example.com");
    assert_eq!(me.body["role"], "member");

    let updated = app
        .patch(
            "/api/me",
            json!({ "name": "  Ann Lee  ", "job_title": "Engineer" }),
            &ann,
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["name"], "Ann Lee");
    assert_eq!(updated.body["job_title"], "Engineer");

    // Email changes from the profile endpoint are ignored.
    let res = app
        .patch("/api/me", json!({ "email": "evil@example.com" }), &ann)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["email"], "ann@example.com");

    // Invalid name.
    assert_eq!(
        app.patch("/api/me", json!({ "name": "   " }), &ann)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    // Clearing the job title.
    let cleared = app.patch("/api/me", json!({ "job_title": "" }), &ann).await;
    assert!(cleared.body["job_title"].is_null());
}

#[tokio::test]
async fn password_change_flow() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    app.create_user(&admin, "pw@test.local", "oldpass123", "member")
        .await;
    let s = app.login("pw@test.local", "oldpass123").await;

    let wrong = app
        .put(
            "/api/me/password",
            json!({ "current_password": "nope", "new_password": "newpass123" }),
            &s,
        )
        .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    let short = app
        .put(
            "/api/me/password",
            json!({ "current_password": "oldpass123", "new_password": "short" }),
            &s,
        )
        .await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST);

    let ok = app
        .put(
            "/api/me/password",
            json!({ "current_password": "oldpass123", "new_password": "newpass123" }),
            &s,
        )
        .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);
    // This browser receives a fresh session and stays signed in.
    let fresh = session_from(&ok.headers).unwrap();
    assert_eq!(app.get("/api/me", &fresh).await.status, StatusCode::OK);

    assert_eq!(
        app.try_login("pw@test.local", "oldpass123").await.status,
        StatusCode::UNAUTHORIZED
    );
    app.login("pw@test.local", "newpass123").await;
}

#[tokio::test]
async fn avatar_upload_validation_and_removal() {
    let app = TestApp::new().await;
    let admin = app.admin().await;

    let upload = |bytes: Vec<u8>, content_type: &'static str| {
        let boundary = "XBOUNDARYX";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a\"\r\n\
             Content-Type: {content_type}\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        Request::post("/api/me/avatar")
            .header(header::COOKIE, &admin.cookie)
            .header("x-csrf-token", &admin.csrf)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap()
    };

    // A text file pretending to be a PNG.
    let fake = app.send(upload(b"hello".to_vec(), "image/png")).await;
    assert_eq!(fake.status, StatusCode::BAD_REQUEST);
    // Unsupported type.
    let svg = app.send(upload(b"<svg/>".to_vec(), "image/svg+xml")).await;
    assert_eq!(svg.status, StatusCode::BAD_REQUEST);
    // Too large.
    let mut big = vec![0x89, b'P', b'N', b'G'];
    big.resize(2 * 1024 * 1024 + 10, 0);
    assert_eq!(
        app.send(upload(big, "image/png")).await.status,
        StatusCode::BAD_REQUEST
    );

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[0; 32]);
    let ok = app.send(upload(png, "image/png")).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    let url = ok.body["avatar_url"].as_str().unwrap().to_string();
    assert!(url.starts_with("/uploads/avatars/"));

    let served = app
        .send(Request::get(url.as_str()).body(Body::empty()).unwrap())
        .await;
    assert_eq!(served.status, StatusCode::OK);

    let removed = app.delete("/api/me/avatar", &admin).await;
    assert_eq!(removed.status, StatusCode::OK);
    assert!(removed.body["avatar_url"].is_null());
    let gone = app
        .send(Request::get(url.as_str()).body(Body::empty()).unwrap())
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_user_management() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let project = app.create_project(&admin, "Onboarding").await;

    let created = app
        .post(
            "/api/admin/users",
            json!({
                "email": "  New.Hire@Example.com ",
                "name": "New Hire",
                "password": "hirepass1",
                "job_title": "Designer",
                "project_ids": [project]
            }),
            &admin,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body["email"], "new.hire@example.com");
    assert_eq!(created.body["role"], "member");
    let id = created.id();

    // Duplicate email (any case).
    let dup = app
        .post(
            "/api/admin/users",
            json!({ "email": "NEW.HIRE@example.com", "name": "X", "password": "hirepass1" }),
            &admin,
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    // Validation errors.
    for body in [
        json!({ "email": "bad", "name": "X", "password": "hirepass1" }),
        json!({ "email": "a@b.co", "name": "", "password": "hirepass1" }),
        json!({ "email": "a@b.co", "name": "X", "password": "short" }),
        json!({ "email": "a@b.co", "name": "X", "password": "hirepass1", "role": "root" }),
        json!({ "email": "a@b.co", "name": "X", "password": "hirepass1", "project_ids": [uuid::Uuid::new_v4()] }),
    ] {
        let res = app.post("/api/admin/users", body.clone(), &admin).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{body}: {}", res.body);
    }
    // The failed create with an unknown project left no user behind.
    assert_eq!(
        app.get("/api/admin/users", &admin)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let detail = app.get(&format!("/api/admin/users/{id}"), &admin).await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body["user"]["project_count"], 1);
    assert_eq!(detail.body["projects"][0]["id"], project);

    // Admin changes email, role, job title; resets password.
    let upd = app
        .patch(
            &format!("/api/admin/users/{id}"),
            json!({ "email": "renamed@example.com", "role": "admin", "job_title": "Lead", "password": "resetpass1" }),
            &admin,
        )
        .await;
    assert_eq!(upd.status, StatusCode::OK, "{}", upd.body);
    assert_eq!(upd.body["email"], "renamed@example.com");
    assert_eq!(upd.body["role"], "admin");
    app.login("renamed@example.com", "resetpass1").await;

    // Email conflict on update.
    let conflict = app
        .patch(
            &format!("/api/admin/users/{id}"),
            json!({ "email": ADMIN_EMAIL }),
            &admin,
        )
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT);

    // Deactivate, then reactivate.
    let off = app
        .patch(
            &format!("/api/admin/users/{id}"),
            json!({ "is_active": false }),
            &admin,
        )
        .await;
    assert_eq!(off.body["is_active"], false);
    let on = app
        .patch(
            &format!("/api/admin/users/{id}"),
            json!({ "is_active": true }),
            &admin,
        )
        .await;
    assert_eq!(on.body["is_active"], true);

    // Unknown user.
    let missing = uuid::Uuid::new_v4();
    assert_eq!(
        app.get(&format!("/api/admin/users/{missing}"), &admin)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&format!("/api/admin/users/{missing}"), &admin)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // Delete removes the user, memberships and entries.
    assert_eq!(
        app.delete(&format!("/api/admin/users/{id}"), &admin)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let mut conn = app.conn().await;
    let left = db::query_i64(
        &mut conn,
        "SELECT (SELECT COUNT(*) FROM users WHERE id = $1) + (SELECT COUNT(*) FROM project_members WHERE user_id = $1)",
        args![uuid::Uuid::parse_str(&id).unwrap()],
    )
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn admin_project_management() {
    let app = TestApp::new().await;
    let admin = app.admin().await;

    let created = app
        .post(
            "/api/admin/projects",
            json!({ "name": " Website ", "client": "ACME", "description": "Redesign", "color": "#AABBCC" }),
            &admin,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["name"], "Website");
    assert_eq!(created.body["color"], "#aabbcc");
    assert_eq!(created.body["is_archived"], false);
    let id = created.id();

    for body in [
        json!({ "name": "" }),
        json!({ "name": "X", "color": "blue" }),
    ] {
        assert_eq!(
            app.post("/api/admin/projects", body, &admin).await.status,
            StatusCode::BAD_REQUEST
        );
    }

    let upd = app
        .patch(
            &format!("/api/admin/projects/{id}"),
            json!({ "name": "Website v2", "is_archived": true, "client": "" }),
            &admin,
        )
        .await;
    assert_eq!(upd.status, StatusCode::OK);
    assert_eq!(upd.body["name"], "Website v2");
    assert_eq!(upd.body["is_archived"], true);
    assert!(upd.body["client"].is_null());
    assert_eq!(upd.body["description"], "Redesign");

    let list = app.get("/api/admin/projects", &admin).await;
    assert_eq!(list.body[0]["member_count"], 0);
    assert_eq!(list.body[0]["total_minutes"], 0);

    let user = app
        .create_user(&admin, "m@test.local", "memberpass1", "member")
        .await;
    app.add_member(&admin, &id, &user).await;
    // Adding again is idempotent.
    app.add_member(&admin, &id, &user).await;
    let detail = app.get(&format!("/api/admin/projects/{id}"), &admin).await;
    assert_eq!(detail.body["project"]["member_count"], 1);
    assert_eq!(detail.body["members"][0]["email"], "m@test.local");

    // Unknown user / project.
    let res = app
        .post(
            &format!("/api/admin/projects/{id}/members"),
            json!({ "user_ids": [uuid::Uuid::new_v4()] }),
            &admin,
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = app
        .post(
            &format!("/api/admin/projects/{}/members", uuid::Uuid::new_v4()),
            json!({ "user_ids": [user] }),
            &admin,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    assert_eq!(
        app.delete(&format!("/api/admin/projects/{id}/members/{user}"), &admin)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.delete(&format!("/api/admin/projects/{id}"), &admin)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.get(&format!("/api/admin/projects/{id}"), &admin)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.delete(&format!("/api/admin/projects/{id}"), &admin)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn timesheet_create_edit_delete() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_user, project, s) = app.member_on_project(&admin, "t@test.local").await;
    let day = today();

    let created = app
        .log_time(&s, &project, day, 2 * 60 + 30, "  Built the login page  ")
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body["minutes"], 150);
    assert_eq!(created.body["description"], "Built the login page");
    assert_eq!(created.body["entry_date"], day.to_string());
    assert_eq!(created.body["project_name"], "Project for t@test.local");
    let id = created.id();

    let mine = app.get("/api/projects", &s).await;
    assert_eq!(mine.body.as_array().unwrap().len(), 1);

    let edited = app
        .patch(
            &format!("/api/time-entries/{id}"),
            json!({ "project_id": project, "entry_date": day.to_string(), "minutes": 45, "description": "Fixed" }),
            &s,
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.body["minutes"], 45);
    assert_eq!(edited.body["description"], "Fixed");

    let list = app
        .get(&format!("/api/time-entries?from={day}&to={day}"), &s)
        .await;
    assert_eq!(total_minutes(&list.body), 45);

    assert_eq!(
        app.delete(&format!("/api/time-entries/{id}"), &s)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.delete(&format!("/api/time-entries/{id}"), &s)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let mut conn = app.conn().await;
    assert_eq!(
        db::query_i64(&mut conn, "SELECT COUNT(*) FROM time_entries", args![])
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn timesheet_input_validation() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_u, project, s) = app.member_on_project(&admin, "v@test.local").await;
    let day = today();

    for minutes in [0, -30, 1441] {
        let res = app.log_time(&s, &project, day, minutes, "").await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{minutes}");
    }
    let res = app.log_time(&s, &project, day, 60, &"x".repeat(5001)).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = app
        .log_time(&s, &project, day + Duration::days(2), 60, "")
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    // Tomorrow is allowed (time zones ahead of UTC).
    let res = app
        .log_time(&s, &project, day + Duration::days(1), 60, "")
        .await;
    assert_eq!(res.status, StatusCode::CREATED);

    // Type errors are rejected by the JSON extractor.
    for body in [
        json!({ "project_id": project, "entry_date": "2026-02-30", "minutes": 60 }),
        json!({ "project_id": project, "entry_date": day.to_string(), "minutes": "60" }),
        json!({ "project_id": "not-a-uuid", "entry_date": day.to_string(), "minutes": 60 }),
        json!({ "entry_date": day.to_string(), "minutes": 60 }),
    ] {
        let res = app.post("/api/time-entries", body.clone(), &s).await;
        assert!(res.status.is_client_error(), "{body}: {}", res.status);
        assert_ne!(res.status, StatusCode::CREATED);
    }
    // Bad query parameters.
    assert_eq!(
        app.get("/api/time-entries?from=2026-02-01&to=2026-01-01", &s)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/time-entries?from=2020-01-01&to=2026-01-01", &s)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert!(app
        .get("/api/time-entries/x?from=bad", &s)
        .await
        .status
        .is_client_error());
}

/// Regression: a user's total per day can never exceed 24 hours.
#[tokio::test]
async fn daily_24_hour_cap() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (user, project, s) = app.member_on_project(&admin, "cap@test.local").await;
    let other = app.create_project(&admin, "Second").await;
    app.add_member(&admin, &other, &user).await;
    let day = today() - Duration::days(1);

    assert_eq!(
        app.log_time(&s, &project, day, 1000, "").await.status,
        StatusCode::CREATED
    );
    // The cap is across projects.
    let over = app.log_time(&s, &other, day, 441, "").await;
    assert_eq!(over.status, StatusCode::BAD_REQUEST);
    assert!(over.error().contains("24 hours"));
    let exact = app.log_time(&s, &other, day, 440, "").await;
    assert_eq!(exact.status, StatusCode::CREATED);
    // Another day is unaffected.
    assert_eq!(
        app.log_time(&s, &project, day - Duration::days(1), 1440, "")
            .await
            .status,
        StatusCode::CREATED
    );
    // Editing an entry excludes its own previous minutes from the total.
    let id = exact.id();
    let edit = app
        .patch(
            &format!("/api/time-entries/{id}"),
            json!({ "project_id": other, "entry_date": day.to_string(), "minutes": 440 }),
            &s,
        )
        .await;
    assert_eq!(edit.status, StatusCode::OK);
    let edit = app
        .patch(
            &format!("/api/time-entries/{id}"),
            json!({ "project_id": other, "entry_date": day.to_string(), "minutes": 441 }),
            &s,
        )
        .await;
    assert_eq!(edit.status, StatusCode::BAD_REQUEST);
}

/// Sends `count` simultaneous entries of `minutes` for one user and day (on a multi-threaded
/// runtime, so they really run in parallel). Returns how many were created.
async fn race(
    app: &TestApp,
    s: &Session,
    project: &str,
    day: NaiveDate,
    count: usize,
    minutes: i64,
) -> usize {
    let barrier = Arc::new(tokio::sync::Barrier::new(count));
    let mut tasks = Vec::new();
    for _ in 0..count {
        let router = app.router.clone();
        let barrier = barrier.clone();
        let req = build(
            Method::POST,
            "/api/time-entries",
            Some(
                json!({ "project_id": project, "entry_date": day.to_string(), "minutes": minutes }),
            ),
            Some(s),
            true,
        );
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            send_to(&router, req).await.status
        }));
    }
    let mut created = 0;
    for t in tasks {
        let status = t.await.unwrap();
        if status == StatusCode::CREATED {
            created += 1;
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
    }
    created
}

/// Regression + concurrency: simultaneous submissions can't overshoot the daily cap
/// (protected by a per-user row lock inside the transaction).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn daily_cap_holds_under_concurrent_requests() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_u, project, s) = app.member_on_project(&admin, "race@test.local").await;

    let day = today();
    assert_eq!(
        race(&app, &s, &project, day, 12, 180).await,
        8,
        "1440 / 180 = 8 entries fit"
    );
    let list = app
        .get(&format!("/api/time-entries?from={day}&to={day}"), &s)
        .await;
    assert_eq!(total_minutes(&list.body), 1440);

    // All requests read the same empty day at once: exactly one 1000-minute entry may win.
    let day = today() - Duration::days(1);
    assert_eq!(race(&app, &s, &project, day, 16, 1000).await, 1);
    let list = app
        .get(&format!("/api/time-entries?from={day}&to={day}"), &s)
        .await;
    assert_eq!(total_minutes(&list.body), 1000);
}

/// Regression: only active projects the user belongs to can receive time.
#[tokio::test]
async fn project_membership_and_archive_rules() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (user, project, s) = app.member_on_project(&admin, "rules@test.local").await;
    let foreign = app.create_project(&admin, "Not mine").await;
    let day = today();

    let res = app.log_time(&s, &foreign, day, 60, "").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert!(res.error().contains("not a member"));

    let entry = app.log_time(&s, &project, day, 60, "before archive").await;
    assert_eq!(entry.status, StatusCode::CREATED);

    app.patch(
        &format!("/api/admin/projects/{project}"),
        json!({ "is_archived": true }),
        &admin,
    )
    .await;
    let res = app.log_time(&s, &project, day, 30, "").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert!(res.error().contains("archived"));
    // Existing entries on an archived project can't be edited either.
    let res = app
        .patch(
            &format!("/api/time-entries/{}", entry.id()),
            json!({ "project_id": project, "entry_date": day.to_string(), "minutes": 90 }),
            &s,
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    // ...but they still show up in reports.
    let list = app
        .get(&format!("/api/time-entries?from={day}&to={day}"), &s)
        .await;
    assert_eq!(total_minutes(&list.body), 60);

    // Moving an entry onto a project you're not in is rejected.
    app.patch(
        &format!("/api/admin/projects/{project}"),
        json!({ "is_archived": false }),
        &admin,
    )
    .await;
    let res = app
        .patch(
            &format!("/api/time-entries/{}", entry.id()),
            json!({ "project_id": foreign, "entry_date": day.to_string(), "minutes": 60 }),
            &s,
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    // Regression: removing a member keeps their history, but blocks new time.
    app.delete(
        &format!("/api/admin/projects/{project}/members/{user}"),
        &admin,
    )
    .await;
    let res = app.log_time(&s, &project, day, 30, "").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let report = app
        .get(
            &format!("/api/admin/time-entries?from={day}&to={day}&project_id={project}"),
            &admin,
        )
        .await;
    assert_eq!(report.body.as_array().unwrap().len(), 1);
    assert_eq!(report.body[0]["description"], "before archive");
    let detail = app
        .get(&format!("/api/admin/projects/{project}"), &admin)
        .await;
    assert_eq!(detail.body["project"]["member_count"], 0);
    assert_eq!(detail.body["project"]["total_minutes"], 60);
}

/// Regression: deleting a project or user removes their entries (cascade).
#[tokio::test]
async fn deleting_projects_and_users_cascades() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (user, project, s) = app.member_on_project(&admin, "casc@test.local").await;
    app.log_time(&s, &project, today(), 60, "").await;
    let second = app.create_project(&admin, "Second").await;
    app.add_member(&admin, &second, &user).await;
    app.log_time(&s, &second, today(), 60, "").await;

    assert_eq!(count_entries(&app).await, 2);
    app.delete(&format!("/api/admin/projects/{project}"), &admin)
        .await;
    assert_eq!(count_entries(&app).await, 1);
    app.delete(&format!("/api/admin/users/{user}"), &admin)
        .await;
    assert_eq!(count_entries(&app).await, 0);
    assert!(verify_ok(&app).await);
}

async fn count_entries(app: &TestApp) -> i64 {
    let mut conn = app.conn().await;
    db::query_i64(&mut conn, "SELECT COUNT(*) FROM time_entries", args![])
        .await
        .unwrap()
}

async fn verify_ok(app: &TestApp) -> bool {
    hourwise_backend::verify::verify(&app.state.db)
        .await
        .unwrap()
        .ok()
}

/// Daily, weekly and monthly totals, plus admin filters and the dashboard stats.
#[tokio::test]
async fn reports_and_totals() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (alice, p1, a) = app.member_on_project(&admin, "alice@test.local").await;
    let (bob, p2, b) = app.member_on_project(&admin, "bob@test.local").await;
    app.add_member(&admin, &p1, &bob).await;

    // A fixed past month crossing a week boundary: Mon 2026-03-30 .. Thu 2026-04-02.
    let entries = [
        (&a, &p1, "2026-03-29", 60),  // Sunday of the previous week
        (&a, &p1, "2026-03-30", 90),  // Monday
        (&a, &p1, "2026-03-31", 45),  // last day of March
        (&a, &p1, "2026-04-01", 120), // first day of April
        (&b, &p2, "2026-03-31", 30),
        (&b, &p1, "2026-04-02", 15),
    ];
    for (s, p, d, m) in entries {
        let res = app.log_time(s, p, date(d), m, "work").await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }

    let appr = &app;
    let get_total = |s: &Session, from: &str, to: &str| {
        let path = format!("/api/time-entries?from={from}&to={to}");
        let s = s.clone();
        async move { total_minutes(&appr.get(&path, &s).await.body) }
    };
    // Day.
    assert_eq!(get_total(&a, "2026-03-31", "2026-03-31").await, 45);
    // Week (Mon–Sun) excludes the previous Sunday.
    assert_eq!(get_total(&a, "2026-03-30", "2026-04-05").await, 255);
    // Months.
    assert_eq!(get_total(&a, "2026-03-01", "2026-03-31").await, 195);
    assert_eq!(get_total(&a, "2026-04-01", "2026-04-30").await, 120);
    // Members only ever see their own time.
    assert_eq!(get_total(&b, "2026-03-01", "2026-04-30").await, 45);

    // Results are newest first.
    let list = app
        .get("/api/time-entries?from=2026-03-01&to=2026-04-30", &a)
        .await;
    let dates: Vec<_> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_date"].as_str().unwrap().to_string())
        .collect();
    let mut sorted = dates.clone();
    sorted.sort_by(|x, y| y.cmp(x));
    assert_eq!(dates, sorted);

    // Admin filters.
    let all = app
        .get(
            "/api/admin/time-entries?from=2026-03-01&to=2026-04-30",
            &admin,
        )
        .await;
    assert_eq!(total_minutes(&all.body), 360);
    let by_project = app
        .get(
            &format!("/api/admin/time-entries?from=2026-03-01&to=2026-04-30&project_id={p1}"),
            &admin,
        )
        .await;
    assert_eq!(total_minutes(&by_project.body), 330);
    let by_user = app
        .get(
            &format!("/api/admin/time-entries?from=2026-03-01&to=2026-04-30&user_id={bob}"),
            &admin,
        )
        .await;
    assert_eq!(total_minutes(&by_user.body), 45);
    let both = app
        .get(&format!("/api/admin/time-entries?from=2026-03-01&to=2026-04-30&user_id={bob}&project_id={p1}"), &admin)
        .await;
    assert_eq!(total_minutes(&both.body), 15);
    // Fields the CSV export relies on are present.
    let e = &both.body[0];
    for field in [
        "entry_date",
        "user_name",
        "user_email",
        "project_client",
        "project_name",
        "minutes",
        "description",
    ] {
        assert!(e.get(field).is_some(), "missing {field}");
    }

    // Per-user and per-project totals.
    let users = app.get("/api/admin/users", &admin).await;
    let alice_row = users
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == alice.as_str())
        .unwrap()
        .clone();
    assert_eq!(alice_row["total_minutes"], 315);
    assert_eq!(alice_row["project_count"], 1);
    let detail = app.get(&format!("/api/admin/projects/{p1}"), &admin).await;
    assert_eq!(detail.body["project"]["total_minutes"], 330);
    assert_eq!(detail.body["project"]["member_count"], 2);
    let bob_member = detail.body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == bob.as_str())
        .unwrap()
        .clone();
    assert_eq!(bob_member["total_minutes"], 15);
}

#[tokio::test]
async fn dashboard_stats_use_current_day_week_and_month() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_u, p, s) = app.member_on_project(&admin, "stats@test.local").await;
    let t = today();
    let week_start = t - Duration::days(i64::from(t.weekday().num_days_from_monday()));
    let month_start = t.with_day(1).unwrap();

    app.log_time(&s, &p, t, 60, "").await;
    let mut expected_week = 60;
    let mut expected_month = 60;
    for d in [week_start, month_start, t - Duration::days(40)] {
        if d == t {
            continue;
        }
        assert_eq!(
            app.log_time(&s, &p, d, 10, "").await.status,
            StatusCode::CREATED
        );
        if d >= week_start {
            expected_week += 10;
        }
        if d >= month_start {
            expected_month += 10;
        }
    }
    // The archived project still counts as a project, but not as active.
    let archived = app.create_project(&admin, "Old").await;
    app.patch(
        &format!("/api/admin/projects/{archived}"),
        json!({ "is_archived": true }),
        &admin,
    )
    .await;

    let stats = app.get("/api/admin/stats", &admin).await;
    assert_eq!(stats.status, StatusCode::OK);
    assert_eq!(stats.body["users"], 2);
    assert_eq!(stats.body["active_users"], 2);
    assert_eq!(stats.body["projects"], 2);
    assert_eq!(stats.body["active_projects"], 1);
    assert_eq!(stats.body["minutes_today"], 60);
    assert_eq!(stats.body["minutes_this_week"], expected_week);
    assert_eq!(stats.body["minutes_this_month"], expected_month);
}

#[tokio::test]
async fn data_persists_after_restart() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_u, p, s) = app.member_on_project(&admin, "persist@test.local").await;
    app.log_time(&s, &p, today(), 75, "persisted").await;

    let restarted = app.restart().await;
    let s2 = restarted.login("persist@test.local", "memberpass1").await;
    let list = restarted
        .get(&format!("/api/time-entries?from={0}&to={0}", today()), &s2)
        .await;
    assert_eq!(total_minutes(&list.body), 75);
    assert_eq!(list.body[0]["description"], "persisted");
    // The old session (signed with the same secret) is still valid after a restart.
    assert_eq!(restarted.get("/api/me", &s).await.status, StatusCode::OK);
}

#[tokio::test]
async fn unicode_and_multiline_notes_roundtrip() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_u, p, s) = app.member_on_project(&admin, "uni@test.local").await;
    let note = "Réunion 会议 😀\nline two\t\"quoted\" 'single'";
    let res = app.log_time(&s, &p, today(), 30, note).await;
    assert_eq!(res.body["description"], note);
    let list = app
        .get(&format!("/api/time-entries?from={0}&to={0}", today()), &s)
        .await;
    assert_eq!(list.body[0]["description"], note);
}
