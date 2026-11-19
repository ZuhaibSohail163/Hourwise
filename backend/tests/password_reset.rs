//! `hourwise-backend reset-password`: the administrator recovery command's database side.
#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

use hourwise_backend::{args, db, error::AppError, password_reset::reset_password};

/// Every column of every user, so a test can prove that nothing else changed.
async fn users_snapshot(app: &TestApp) -> Vec<String> {
    let mut conn = app.conn().await;
    db::query_records(
        &mut conn,
        "SELECT to_jsonb(u)::text AS row FROM users u ORDER BY email",
        args![],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.text("row").unwrap())
    .collect()
}

async fn user_row(app: &TestApp, email: &str) -> (String, i64, String) {
    let mut conn = app.conn().await;
    let rows = db::query_records(
        &mut conn,
        "SELECT password_hash, session_epoch, updated_at::text AS updated FROM users WHERE email = $1",
        args![email],
    )
    .await
    .unwrap();
    let r = &rows[0];
    (
        r.text("password_hash").unwrap(),
        r.i64("session_epoch").unwrap(),
        r.text("updated").unwrap(),
    )
}

#[tokio::test]
async fn resets_only_the_password_hash_updated_at_and_session_epoch() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    app.create_user(&admin, "other@test.local", "otherpass1", "member")
        .await;
    let before = users_snapshot(&app).await;
    let (old_hash, old_epoch, old_updated) = user_row(&app, ADMIN_EMAIL).await;

    // The email is matched the way login matches it (trimmed, case-insensitive).
    let outcome = reset_password(
        &app.state.db,
        &format!("  {}  ", ADMIN_EMAIL.to_uppercase()),
        "Recovered-Pass-42".into(),
    )
    .await
    .unwrap();
    assert_eq!(outcome.email, ADMIN_EMAIL);
    assert_eq!(outcome.role, "admin");
    assert!(outcome.is_active);

    let (hash, epoch, updated) = user_row(&app, ADMIN_EMAIL).await;
    assert_ne!(hash, old_hash);
    assert_eq!(epoch, old_epoch + 1);
    assert_ne!(updated, old_updated);
    // The application's Argon2id settings, and no trace of the plaintext.
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "unexpected hash parameters"
    );
    assert!(!hash.contains("Recovered-Pass-42"));

    // Nothing else changed: the other user is identical, and the admin row differs only in the
    // three columns the command owns.
    let after = users_snapshot(&app).await;
    assert_eq!(before.len(), after.len());
    let strip = |row: &str| {
        let mut v: serde_json::Value = serde_json::from_str(row).unwrap();
        for k in ["password_hash", "updated_at", "session_epoch"] {
            v.as_object_mut().unwrap().remove(k);
        }
        v
    };
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(strip(b), strip(a));
    }
    assert!(
        before.iter().any(|r| after.contains(r)),
        "the other user must be untouched"
    );

    // The new password works through the normal login; the old one doesn't.
    assert_eq!(
        app.try_login(ADMIN_EMAIL, ADMIN_PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    let s = app.login(ADMIN_EMAIL, "Recovered-Pass-42").await;
    assert_eq!(app.get("/api/me", &s).await.body["role"], "admin");
}

#[tokio::test]
async fn existing_sessions_of_that_account_are_signed_out() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    app.create_user(&admin, "bystander@test.local", "bystander1", "member")
        .await;
    let bystander = app.login("bystander@test.local", "bystander1").await;
    assert_eq!(app.get("/api/me", &admin).await.status, StatusCode::OK);

    reset_password(&app.state.db, ADMIN_EMAIL, "Recovered-Pass-42".into())
        .await
        .unwrap();

    assert_eq!(
        app.get("/api/me", &admin).await.status,
        StatusCode::UNAUTHORIZED
    );
    // Other accounts keep their sessions.
    assert_eq!(app.get("/api/me", &bystander).await.status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_account_changes_nothing() {
    let app = TestApp::new().await;
    let before = users_snapshot(&app).await;

    let err = reset_password(&app.state.db, "nobody@test.local", "Valid-Pass-123".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
    assert_eq!(users_snapshot(&app).await, before);
}

#[tokio::test]
async fn invalid_passwords_are_rejected_before_anything_changes() {
    let app = TestApp::new().await;
    let before = users_snapshot(&app).await;

    for bad in ["", "short", "1234567"] {
        let err = reset_password(&app.state.db, ADMIN_EMAIL, bad.into())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert!(!err.detail().contains(bad) || bad.is_empty());
    }
    let err = reset_password(&app.state.db, "   ", "Valid-Pass-123".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");

    assert_eq!(users_snapshot(&app).await, before);
    // The old password still works.
    app.login(ADMIN_EMAIL, ADMIN_PASSWORD).await;
}

#[tokio::test]
async fn a_database_failure_changes_nothing() {
    let app = TestApp::new().await;
    let before = users_snapshot(&app).await;

    let closed = app.database.connect().await;
    closed.close().await;
    let err = reset_password(&closed, ADMIN_EMAIL, "Valid-Pass-123".into())
        .await
        .unwrap_err();
    assert!(
        matches!(err, AppError::Unavailable(_) | AppError::Internal(_)),
        "{err:?}"
    );
    assert_eq!(users_snapshot(&app).await, before);
}

#[tokio::test]
async fn a_deactivated_account_stays_deactivated() {
    let app = TestApp::new().await;
    let admin = app.admin().await;
    let id = app
        .create_user(&admin, "gone@test.local", "gonepass1", "member")
        .await;
    let res = app
        .patch(
            &format!("/api/admin/users/{id}"),
            json!({ "is_active": false }),
            &admin,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);

    let outcome = reset_password(&app.state.db, "gone@test.local", "Fresh-Pass-123".into())
        .await
        .unwrap();
    assert!(!outcome.is_active);
    // Right password, but the account is still deactivated (the command doesn't reactivate).
    let res = app.try_login("gone@test.local", "Fresh-Pass-123").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert!(res.error().contains("deactivated"));
}
