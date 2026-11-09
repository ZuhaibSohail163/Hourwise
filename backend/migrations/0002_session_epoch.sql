-- Session revocation: every session token carries the user's epoch at login time. Bumping the
-- epoch (password change, admin password reset, deactivation) invalidates all existing sessions.
ALTER TABLE users ADD COLUMN session_epoch INTEGER NOT NULL DEFAULT 0;
