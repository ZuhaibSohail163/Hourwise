//! `hourwise-backend reset-password <email>`: one-off administrator recovery from the server.
//!
//! The new password arrives on standard input (never as an argument, so it stays out of shell
//! history and the process list). It is checked with the same rules as the API and hashed with
//! the same Argon2id settings. Only that user's `password_hash`, `updated_at` and `session_epoch`
//! change, in one transaction that must touch exactly one row; bumping `session_epoch` signs the
//! account out everywhere. Nothing here runs at startup or during a deploy.

use crate::{
    args,
    auth::{self, validate_password},
    db::{self, Db},
    error::{AppError, AppResult},
};

/// Larger input is refused rather than read: a password never needs this much.
pub const MAX_INPUT_BYTES: usize = 4096;

/// What was changed, for the operator's confirmation message (no secrets).
#[derive(Debug, PartialEq, Eq)]
pub struct ResetOutcome {
    pub email: String,
    pub role: String,
    pub is_active: bool,
}

/// Turns the bytes read from stdin into the password: one line, with its line ending removed.
/// Everything else (including leading or trailing spaces) is part of the password, exactly as
/// the login form would send it.
pub fn password_from_input(input: &[u8]) -> Result<String, String> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "input is larger than {MAX_INPUT_BYTES} bytes; send only the password"
        ));
    }
    let text = std::str::from_utf8(input).map_err(|_| "the password is not valid UTF-8")?;
    let line = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if line.contains(['\n', '\r']) {
        return Err("send the password as a single line".into());
    }
    if line.is_empty() {
        return Err("no password received on standard input".into());
    }
    Ok(line.to_string())
}

/// Sets a new password for the account with this email and revokes its sessions.
///
/// Validation and hashing happen before the database is touched. The update runs in one
/// transaction that locks the row and must change exactly one row; any error rolls it back.
pub async fn reset_password(db: &Db, email: &str, password: String) -> AppResult<ResetOutcome> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::bad("an email address is required"));
    }
    validate_password(&password)?;
    let hash = auth::hash_password(password).await?;

    let mut conn = db.conn().await?;
    let mut tx = db::write_tx(&mut conn).await?;
    let row = db::query_records(
        &mut tx,
        "SELECT id, role, is_active FROM users WHERE email = $1 FOR UPDATE",
        args![&email],
    )
    .await?;
    let [user] = row.as_slice() else {
        // Dropping `tx` rolls back; nothing was written.
        return Err(AppError::NotFound(format!("no account with email {email}")));
    };
    let id = user.uuid("id")?;
    let outcome = ResetOutcome {
        email: email.clone(),
        role: user.text("role")?,
        is_active: user.bool("is_active")?,
    };

    let changed = db::execute(
        &mut tx,
        "UPDATE users
         SET password_hash = $2, updated_at = $3, session_epoch = session_epoch + 1
         WHERE id = $1",
        args![id, hash, db::now_ts()],
    )
    .await?;
    if changed != 1 {
        return Err(AppError::Internal(format!(
            "expected to update 1 account, updated {changed}; rolled back"
        )));
    }
    tx.commit().await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_one_line_and_strips_only_its_line_ending() {
        let ok = |s: &str| Ok(s.to_string());
        assert_eq!(password_from_input(b"secret123\n"), ok("secret123"));
        assert_eq!(password_from_input(b"secret123\r\n"), ok("secret123"));
        assert_eq!(password_from_input(b"secret123"), ok("secret123"));
        assert_eq!(password_from_input(b" pass word \n"), ok(" pass word "));
    }

    #[test]
    fn rejects_empty_multiline_oversized_and_non_utf8_input() {
        assert!(password_from_input(b"").is_err());
        assert!(password_from_input(b"\n").is_err());
        assert!(password_from_input(b"one\ntwo\n").is_err());
        assert!(password_from_input(b"one\rtwo").is_err());
        assert!(password_from_input(&[0xff, 0xfe]).is_err());
        assert!(password_from_input(&vec![b'a'; MAX_INPUT_BYTES + 1]).is_err());
    }

    #[test]
    fn errors_never_echo_the_input() {
        for input in [&b"one\nsecret-two\n"[..], &[b'x'; MAX_INPUT_BYTES + 1][..]] {
            let Err(msg) = password_from_input(input) else {
                panic!("input should be rejected");
            };
            assert!(
                !msg.contains("secret-two") && !msg.contains("xxxx"),
                "{msg}"
            );
        }
    }
}
