//! Authentication, authorization and input-hardening tests.
#![allow(clippy::unwrap_used)]

mod common;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use common::*;
use hourwise_backend::{
    args,
    auth::create_token,
    db::{self, Db, DbTarget},
    models::User,
};
use serde_json::json;

const PROTECTED: &[(&str, &str)] = &[
    ("GET", "/api/me"),
    ("PATCH", "/api/me"),
    ("PUT", "/api/me/password"),
    ("POST", "/api/me/avatar"),
    ("DELETE", "/api/me/avatar"),
    ("GET", "/api/projects"),
    ("GET", "/api/time-entries"),
    ("POST", "/api/time-entries"),
    (
        "PATCH",
        "/api/time-entries/00000000-0000-4000-8000-000000000000",
    ),
    (
        "DELETE",
        "/api/time-entries/00000000-0000-4000-8000-000000000000",
    ),
];

const ADMIN_ONLY: &[(&str, &str)] = &[
    ("GET", "/api/admin/stats"),
    ("GET", "/api/admin/users"),
    ("POST", "/api/admin/users"),
    ("GET", "/api/admin/users/00000000-0000-4000-8000-000000000000"),
    ("PATCH", "/api/admin/users/00000000-0000-4000-8000-000000000000"),
    ("DELETE", "/api/admin/users/00000000-0000-4000-8000-000000000000"),
    ("GET", "/api/admin/projects"),
    ("POST", "/api/admin/projects"),
    ("GET", "/api/admin/projects/00000000-0000-4000-8000-000000000000"),
    ("PATCH", "/api/admin/projects/00000000-0000-4000-8000-000000000000"),
    ("DELETE", "/api/admin/projects/00000000-0000-4000-8000-000000000000"),
    ("POST", "/api/admin/projects/00000000-0000-4000-8000-000000000000/members"),
    (
        "DELETE",
        "/api/admin/projects/00000000-0000-4000-8000-000000000000/members/00000000-0000-4000-8000-000000000000",
    ),
    ("GET", "/api/admin/time-entries"),
];

fn method(m: &str) -> Method {
    m.parse().unwrap()
}

fn body_for(m: &str) -> Option<serde_json::Value> {
    (m != "GET" && m != "DELETE").then(|| json!({}))
}

#[tokio::test]
async fn every_protected_endpoint_requires_a_session() {
    let app = TestApp::new().await;
    for (m, path) in PROTECTED.iter().chain(ADMIN_ONLY) {
        let res = app.call(method(m), path, body_for(m), None).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{m} {path}");
        assert!(res.body["error"].is_string());
    }
}

#[tokio::test]
async fn members_cannot_use_admin_endpoints() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    app.create_user(&admin, "emp@test.local", "emppass123", "member")
        .await;
    let emp = app.login("emp@test.local", "emppass123").await;
    for (m, path) in ADMIN_ONLY {
        let res = app.call(method(m), path, body_for(m), Some(&emp)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{m} {path}");
    }
    // Nothing was created as a side effect.
    let users = app.get("/api/admin/users", &admin).await;
    assert_eq!(users.body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn invalid_tampered_and_forged_sessions_are_rejected() {
    let app = TestApp::new().await;
    let admin = app.admin().await;

    let garbage = Session {
        cookie: "hw_session=not-a-jwt; hw_csrf=x".into(),
        csrf: "x".into(),
    };
    assert_eq!(
        app.get("/api/me", &garbage).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Flip a character in the signature.
    let mut tampered = admin.clone();
    let last = tampered.cookie.find(';').unwrap() - 1;
    let flipped = if &tampered.cookie[last..=last] == "A" {
        "B"
    } else {
        "A"
    };
    tampered.cookie.replace_range(last..=last, flipped);
    assert_eq!(
        app.get("/api/me", &tampered).await.status,
        StatusCode::UNAUTHORIZED
    );

    // A token signed with another secret (e.g. forged admin claims).
    let conn = app.state.db.conn().await.unwrap();
    let user: User = db::query_one(&conn, "SELECT * FROM users LIMIT 1", args![])
        .await
        .unwrap();
    let forged = create_token(&user, "c", "some-other-secret-some-other-secret!!", 1).unwrap();
    let s = Session {
        cookie: format!("hw_session={forged}; hw_csrf=c"),
        csrf: "c".into(),
    };
    assert_eq!(
        app.get("/api/me", &s).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Bearer tokens are not accepted (sessions are cookie-only).
    let token = create_token(&user, "c", JWT_SECRET, 1).unwrap();
    let res = app
        .send(
            Request::get("/api/me")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_sessions_are_rejected() {
    let app = TestApp::new().await;
    app.admin().await;
    let conn = app.state.db.conn().await.unwrap();
    let user: User = db::query_one(&conn, "SELECT * FROM users LIMIT 1", args![])
        .await
        .unwrap();
    let expired = create_token(&user, "c", JWT_SECRET, -1).unwrap();
    let s = Session {
        cookie: format!("hw_session={expired}; hw_csrf=c"),
        csrf: "c".into(),
    };
    assert_eq!(
        app.get("/api/me", &s).await.status,
        StatusCode::UNAUTHORIZED
    );
    let valid = create_token(&user, "c", JWT_SECRET, 1).unwrap();
    let s = Session {
        cookie: format!("hw_session={valid}; hw_csrf=c"),
        csrf: "c".into(),
    };
    assert_eq!(app.get("/api/me", &s).await.status, StatusCode::OK);
}

#[tokio::test]
async fn session_cookie_attributes() {
    let app = TestApp::new().await;
    let res = app.try_login(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let cookies: Vec<String> = res
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let session = cookies
        .iter()
        .find(|c| c.starts_with("hw_session="))
        .unwrap();
    let csrf = cookies.iter().find(|c| c.starts_with("hw_csrf=")).unwrap();
    for attr in [
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=3600",
    ] {
        assert!(
            session.contains(attr),
            "session cookie lacks {attr}: {session}"
        );
    }
    // The CSRF cookie must be readable by the page, so it is not HttpOnly.
    assert!(!csrf.contains("HttpOnly"));
    assert!(csrf.contains("SameSite=Strict") && csrf.contains("Secure"));

    let insecure = TestApp::with(|c| c.cookie_secure = false).await;
    let res = insecure.try_login(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert!(res
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .all(|v| !v.to_str().unwrap().contains("Secure")));
}

#[tokio::test]
async fn csrf_token_is_required_for_state_changes() {
    let app = TestApp::new().await;
    let admin = app.admin().await;

    // Missing header.
    let res = app
        .send(build(
            Method::POST,
            "/api/admin/projects",
            Some(json!({ "name": "CSRF" })),
            Some(&admin),
            false,
        ))
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    // Wrong token (e.g. a token from another session).
    let other = app.admin().await;
    let mixed = Session {
        cookie: admin.cookie.clone(),
        csrf: other.csrf.clone(),
    };
    let res = app
        .post("/api/admin/projects", json!({ "name": "CSRF" }), &mixed)
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    // Nothing was created.
    assert_eq!(
        app.get("/api/admin/projects", &admin)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // GET requests don't need the token.
    let res = app
        .send(build(Method::GET, "/api/me", None, Some(&admin), false))
        .await;
    assert_eq!(res.status, StatusCode::OK);
    // With the right token it works.
    assert_eq!(
        app.post("/api/admin/projects", json!({ "name": "CSRF" }), &admin)
            .await
            .status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn logout_expires_cookies() {
    let app = TestApp::new().await;
    let res = app.call(Method::POST, "/api/auth/logout", None, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let cookies: Vec<_> = res.headers.get_all(header::SET_COOKIE).iter().collect();
    assert_eq!(cookies.len(), 2);
    assert!(cookies
        .iter()
        .all(|c| c.to_str().unwrap().contains("Max-Age=0")));
}

#[tokio::test]
async fn employees_cannot_touch_each_others_entries() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_a, project, alice) = app.member_on_project(&admin, "alice@test.local").await;
    let bob_id = app
        .create_user(&admin, "bob@test.local", "bobpass123", "member")
        .await;
    app.add_member(&admin, &project, &bob_id).await;
    let bob = app.login("bob@test.local", "bobpass123").await;

    let entry = app
        .log_time(&alice, &project, today(), 60, "alice's work")
        .await;
    let id = entry.id();

    let res = app
        .patch(
            &format!("/api/time-entries/{id}"),
            json!({ "project_id": project, "entry_date": today().to_string(), "minutes": 1 }),
            &bob,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(
        app.delete(&format!("/api/time-entries/{id}"), &bob)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // Listing ignores attempts to filter by another user.
    let list = app
        .get(
            &format!(
                "/api/time-entries?user_id={}",
                entry.body["user_id"].as_str().unwrap()
            ),
            &bob,
        )
        .await;
    assert_eq!(list.body.as_array().unwrap().len(), 0);

    // Alice's entry is untouched.
    let mine = app.get("/api/time-entries", &alice).await;
    assert_eq!(mine.body[0]["minutes"], 60);
}

#[tokio::test]
async fn sql_injection_attempts_are_inert() {
    let app = TestApp::new().await;
    let admin = app.admin().await;

    for email in ["' OR '1'='1", "admin@test.local' --", "\" OR 1=1 --"] {
        assert_eq!(
            app.try_login(email, "x' OR '1'='1").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let evil = "'); DROP TABLE users; --";
    let res = app
        .post(
            "/api/admin/projects",
            json!({ "name": evil, "client": "x' OR 1=1 --" }),
            &admin,
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["name"], evil);
    // Query parameters are typed; injection strings don't parse.
    let res = app
        .get("/api/admin/time-entries?project_id=1%20OR%201%3D1", &admin)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = app
        .get("/api/admin/users/1%27%20OR%20%271%27%3D%271", &admin)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    // The users table still exists and is intact.
    let users = app.get("/api/admin/users", &admin).await;
    assert_eq!(users.status, StatusCode::OK);
    assert_eq!(users.body.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn malicious_inputs_are_rejected_or_stored_verbatim() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let (_u, p, s) = app.member_on_project(&admin, "x@test.local").await;

    // HTML/script is stored as plain text (React escapes it on render).
    let xss = "<script>alert(1)</script><img src=x onerror=alert(1)>";
    let res = app.log_time(&s, &p, today(), 30, xss).await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["description"], xss);

    // Oversized bodies are refused before reaching handlers.
    let huge = "x".repeat(6 * 1024 * 1024);
    let res = app
        .post("/api/time-entries", json!({ "description": huge }), &s)
        .await;
    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);

    // Wrong content type.
    let res = app
        .send(
            Request::post("/api/time-entries")
                .header(header::COOKIE, &s.cookie)
                .header("x-csrf-token", &s.csrf)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("minutes=60"))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Integer overflow.
    let res = app
        .post(
            "/api/time-entries",
            json!({ "project_id": p, "entry_date": today().to_string(), "minutes": 99_999_999_999i64 }),
            &s,
        )
        .await;
    assert!(res.status.is_client_error());

    // Path traversal in uploads.
    let res = app
        .send(
            Request::get("/uploads/../hourwise.db")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_ne!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn deactivated_users_are_locked_out_immediately() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let id = app
        .create_user(&admin, "gone@test.local", "gonepass1", "member")
        .await;
    let s = app.login("gone@test.local", "gonepass1").await;

    app.patch(
        &format!("/api/admin/users/{id}"),
        json!({ "is_active": false }),
        &admin,
    )
    .await;
    assert_eq!(
        app.get("/api/me", &s).await.status,
        StatusCode::UNAUTHORIZED
    );
    let res = app.try_login("gone@test.local", "gonepass1").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert!(res.error().contains("deactivated"));
    assert!(session_from(&res.headers).is_none());
    // A wrong password doesn't reveal that the account is deactivated.
    assert_eq!(
        app.try_login("gone@test.local", "wrongpass").await.status,
        StatusCode::UNAUTHORIZED
    );

    // Reactivating does not revive the old session.
    app.patch(
        &format!("/api/admin/users/{id}"),
        json!({ "is_active": true }),
        &admin,
    )
    .await;
    assert_eq!(
        app.get("/api/me", &s).await.status,
        StatusCode::UNAUTHORIZED
    );
    app.login("gone@test.local", "gonepass1").await;
}

#[tokio::test]
async fn role_changes_apply_to_existing_sessions() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let id = app
        .create_user(&admin, "promo@test.local", "promopass1", "member")
        .await;
    let s = app.login("promo@test.local", "promopass1").await;
    assert_eq!(
        app.get("/api/admin/stats", &s).await.status,
        StatusCode::FORBIDDEN
    );
    app.patch(
        &format!("/api/admin/users/{id}"),
        json!({ "role": "admin" }),
        &admin,
    )
    .await;
    assert_eq!(app.get("/api/admin/stats", &s).await.status, StatusCode::OK);
    app.patch(
        &format!("/api/admin/users/{id}"),
        json!({ "role": "member" }),
        &admin,
    )
    .await;
    assert_eq!(
        app.get("/api/admin/stats", &s).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admins_cannot_demote_deactivate_or_delete_themselves() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let me = app.get("/api/me", &admin).await.id();
    for body in [json!({ "role": "member" }), json!({ "is_active": false })] {
        let res = app
            .patch(&format!("/api/admin/users/{me}"), body, &admin)
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        app.delete(&format!("/api/admin/users/{me}"), &admin)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let still = app.get("/api/me", &admin).await;
    assert_eq!(still.body["role"], "admin");
    assert_eq!(still.body["is_active"], true);
}

#[tokio::test]
async fn password_changes_revoke_other_sessions() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let id = app
        .create_user(&admin, "rev@test.local", "revpass123", "member")
        .await;
    let laptop = app.login("rev@test.local", "revpass123").await;
    let phone = app.login("rev@test.local", "revpass123").await;

    let res = app
        .put(
            "/api/me/password",
            json!({ "current_password": "revpass123", "new_password": "revpass456" }),
            &laptop,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get("/api/me", &phone).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get("/api/me", &laptop).await.status,
        StatusCode::UNAUTHORIZED
    );
    let renewed = session_from(&res.headers).unwrap();
    assert_eq!(app.get("/api/me", &renewed).await.status, StatusCode::OK);

    // Admin password reset signs the user out everywhere.
    app.patch(
        &format!("/api/admin/users/{id}"),
        json!({ "password": "adminset99" }),
        &admin,
    )
    .await;
    assert_eq!(
        app.get("/api/me", &renewed).await.status,
        StatusCode::UNAUTHORIZED
    );
    // A profile edit by an admin (no password) does not.
    let s = app.login("rev@test.local", "adminset99").await;
    app.patch(
        &format!("/api/admin/users/{id}"),
        json!({ "name": "Renamed" }),
        &admin,
    )
    .await;
    assert_eq!(app.get("/api/me", &s).await.status, StatusCode::OK);
}

#[tokio::test]
async fn login_rate_limit() {
    let app = TestApp::with(|c| c.login_max_attempts = 3).await;
    for _ in 0..3 {
        assert_eq!(
            app.try_login(ADMIN_EMAIL, "wrong").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let locked = app.try_login(ADMIN_EMAIL, "wrong").await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(locked.headers.get(header::RETRY_AFTER).is_some());
    // Even the correct password is refused while locked.
    assert_eq!(
        app.try_login(ADMIN_EMAIL, ADMIN_PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // The lock is per email, and case-insensitive.
    assert_eq!(
        app.try_login(&ADMIN_EMAIL.to_uppercase(), ADMIN_PASSWORD)
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        app.try_login("other@test.local", "wrong").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn successful_login_resets_the_failure_counter() {
    let app = TestApp::with(|c| c.login_max_attempts = 3).await;
    for _ in 0..2 {
        app.try_login(ADMIN_EMAIL, "wrong").await;
    }
    app.admin().await;
    for _ in 0..2 {
        assert_eq!(
            app.try_login(ADMIN_EMAIL, "wrong").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn database_outage_returns_503_without_leaking_details() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let dead = Db::open(&DbTarget::parse("http://127.0.0.1:9", None).unwrap())
        .await
        .unwrap();
    let router = app.with_db(dead);
    let res = send_to(
        &router,
        build(Method::GET, "/api/me", None, Some(&admin), true),
    )
    .await;
    assert!(
        res.status == StatusCode::SERVICE_UNAVAILABLE
            || res.status == StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        res.status
    );
    let msg = res.error();
    assert!(!msg.contains("127.0.0.1"), "{msg}");
    assert!(!msg.to_lowercase().contains("hrana"), "{msg}");
}

#[tokio::test]
async fn passwords_are_stored_hashed() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    app.create_user(&admin, "hash@test.local", "plaintext-secret", "member")
        .await;
    let conn = app.state.db.conn().await.unwrap();
    let hash = db::query_records(
        &conn,
        "SELECT password_hash FROM users WHERE email = 'hash@test.local'",
        args![],
    )
    .await
    .unwrap()[0]
        .text("password_hash")
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains("plaintext-secret"));
}
