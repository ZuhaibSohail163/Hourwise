-- Hourwise schema for Turso / libSQL (SQLite dialect).
-- Converted from the original PostgreSQL schema (kept in migrations/postgres-legacy/).
--
-- Storage formats (enforced by CHECK constraints, produced by src/db.rs):
--   ids         TEXT  lower-case hyphenated UUID (36 chars), same values as in PostgreSQL
--   booleans    INTEGER 0/1
--   dates       TEXT  'YYYY-MM-DD' (sorts and compares correctly as text)
--   timestamps  TEXT  RFC 3339 UTC with milliseconds, e.g. '2026-11-09T14:03:12.345Z'
--   durations   INTEGER minutes (1..1440 per entry)
-- STRICT tables reject values of the wrong type, like PostgreSQL does.

CREATE TABLE users (
    id            TEXT    PRIMARY KEY CHECK (length(id) = 36),
    email         TEXT    NOT NULL UNIQUE CHECK (email = lower(email) AND length(email) > 0),
    name          TEXT    NOT NULL,
    password_hash TEXT    NOT NULL,
    role          TEXT    NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
    job_title     TEXT,
    avatar_url    TEXT,
    is_active     INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE projects (
    id          TEXT    PRIMARY KEY CHECK (length(id) = 36),
    name        TEXT    NOT NULL,
    client      TEXT,
    description TEXT,
    color       TEXT    NOT NULL DEFAULT '#f36c00',
    is_archived INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE project_members (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    added_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (project_id, user_id)
) STRICT;
CREATE INDEX project_members_user_idx ON project_members (user_id);

CREATE TABLE time_entries (
    id          TEXT    PRIMARY KEY CHECK (length(id) = 36),
    user_id     TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id  TEXT    NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    entry_date  TEXT    NOT NULL
                CHECK (entry_date GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
    minutes     INTEGER NOT NULL CHECK (minutes > 0 AND minutes <= 1440),
    description TEXT    NOT NULL DEFAULT '',
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX time_entries_user_date_idx ON time_entries (user_id, entry_date);
CREATE INDEX time_entries_project_date_idx ON time_entries (project_id, entry_date);
