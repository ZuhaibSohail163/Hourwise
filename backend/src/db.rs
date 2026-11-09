//! Database access on top of the official `libsql` crate.
//!
//! The same code talks to:
//! - **Turso Cloud** (`libsql://<db>-<org>.turso.io` or `https://…`) over HTTPS with an auth token
//! - a **local SQLite file** (`file:./data/hourwise.db`): free, offline, used by default in Docker
//!   and by the test suite
//!
//! SQL is plain SQLite with numbered placeholders (`?1`, `?2`, …). Every value is bound through
//! [`Bind`], never formatted into the SQL string.

use std::{collections::HashMap, path::Path, time::Duration};

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use libsql::{Builder, Connection, Database, Row, TransactionBehavior, Value};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Where the database lives. Parsed from `TURSO_DATABASE_URL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbTarget {
    /// Turso Cloud / any libSQL server (`libsql://`, `https://`, `http://`).
    Remote { url: String, auth_token: String },
    /// A local SQLite database file.
    Local { path: String },
}

impl DbTarget {
    pub fn parse(url: &str, auth_token: Option<String>) -> Result<Self, String> {
        let url = url.trim();
        let is_remote = ["libsql://", "https://", "http://", "wss://", "ws://"]
            .iter()
            .any(|p| url.starts_with(p));
        if is_remote {
            let auth_token = auth_token.unwrap_or_default();
            // http:// is only used for a local `turso dev` / sqld server, which needs no token.
            if auth_token.is_empty() && !url.starts_with("http://") {
                return Err(
                    "TURSO_AUTH_TOKEN must be set when TURSO_DATABASE_URL points at Turso Cloud"
                        .into(),
                );
            }
            return Ok(DbTarget::Remote {
                url: url.to_string(),
                auth_token,
            });
        }
        let path = url
            .strip_prefix("file://")
            .or_else(|| url.strip_prefix("file:"))
            .unwrap_or(url);
        if path.is_empty() || path == ":memory:" {
            return Err(
                "TURSO_DATABASE_URL must be a Turso URL (libsql://…) or a database file path \
                 (file:./data/hourwise.db); in-memory databases are not supported"
                    .into(),
            );
        }
        Ok(DbTarget::Local {
            path: path.to_string(),
        })
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, DbTarget::Remote { .. })
    }

    /// A printable description that never includes the auth token.
    pub fn describe(&self) -> String {
        match self {
            DbTarget::Remote { url, .. } => format!("Turso/libSQL server at {url}"),
            DbTarget::Local { path } => format!("local SQLite file {path}"),
        }
    }
}

/// Shared handle to the database. Cheap to clone; open a [`Connection`] per request with
/// [`Db::conn`].
#[derive(Clone)]
pub struct Db {
    database: std::sync::Arc<Database>,
    remote: bool,
}

impl Db {
    pub async fn open(target: &DbTarget) -> AppResult<Self> {
        let database = match target {
            DbTarget::Remote { url, auth_token } => {
                Builder::new_remote(url.clone(), auth_token.clone())
                    .build()
                    .await?
            }
            DbTarget::Local { path } => {
                if let Some(parent) = Path::new(path).parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            AppError::Internal(format!("cannot create database directory: {e}"))
                        })?;
                    }
                }
                Builder::new_local(path).build().await?
            }
        };
        let db = Db {
            database: std::sync::Arc::new(database),
            remote: target.is_remote(),
        };
        if !db.remote {
            // WAL lets readers proceed while one writer commits. The setting is persistent.
            let conn = db.conn().await?;
            let mut rows = conn.query("PRAGMA journal_mode = WAL", ()).await?;
            while rows.next().await?.is_some() {}
        }
        Ok(db)
    }

    pub fn is_remote(&self) -> bool {
        self.remote
    }

    /// Opens a connection. For local files this sets the per-connection pragmas SQLite needs
    /// (foreign keys are OFF by default in SQLite, and concurrent writers need a busy timeout).
    ///
    /// For Turso Cloud the connection is an HTTPS client; the application does not rely on
    /// `PRAGMA foreign_keys` there and performs cascading deletes explicitly (see `admin.rs`).
    pub async fn conn(&self) -> AppResult<Connection> {
        let conn = self.database.connect()?;
        if !self.remote {
            conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 10000;")
                .await?;
        }
        Ok(conn)
    }

    /// Checks that the database is reachable and the credentials are valid.
    pub async fn ping(&self) -> AppResult<()> {
        let conn = self.conn().await?;
        let one = query_i64(&conn, "SELECT 1", crate::args![]).await?;
        if one != 1 {
            return Err(AppError::Internal("unexpected ping result".into()));
        }
        Ok(())
    }
}

/// Starts a write transaction that takes the write lock up front (`BEGIN IMMEDIATE`), so
/// read-then-write checks (like the 24h daily cap) cannot race with another request.
pub async fn write_tx(conn: &Connection) -> AppResult<libsql::Transaction> {
    Ok(conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?)
}

// ---------------------------------------------------------------- binding values

/// Converts a Rust value into a SQLite value. Formats are part of the schema contract:
/// UUIDs are lower-case hyphenated text, dates are `YYYY-MM-DD`, timestamps are
/// RFC 3339 UTC with milliseconds, booleans are 0/1.
pub trait Bind {
    fn into_value(self) -> Value;
}

impl Bind for Value {
    fn into_value(self) -> Value {
        self
    }
}
impl Bind for String {
    fn into_value(self) -> Value {
        Value::Text(self)
    }
}
impl Bind for &String {
    fn into_value(self) -> Value {
        Value::Text(self.clone())
    }
}
impl Bind for &str {
    fn into_value(self) -> Value {
        Value::Text(self.to_string())
    }
}
impl Bind for i64 {
    fn into_value(self) -> Value {
        Value::Integer(self)
    }
}
impl Bind for i32 {
    fn into_value(self) -> Value {
        Value::Integer(i64::from(self))
    }
}
impl Bind for bool {
    fn into_value(self) -> Value {
        Value::Integer(i64::from(self))
    }
}
impl Bind for Uuid {
    fn into_value(self) -> Value {
        Value::Text(self.hyphenated().to_string())
    }
}
impl Bind for &Uuid {
    fn into_value(self) -> Value {
        (*self).into_value()
    }
}
impl Bind for NaiveDate {
    fn into_value(self) -> Value {
        Value::Text(format_date(self))
    }
}
impl Bind for DateTime<Utc> {
    fn into_value(self) -> Value {
        Value::Text(format_ts(self))
    }
}
impl<T: Bind> Bind for Option<T> {
    fn into_value(self) -> Value {
        match self {
            Some(v) => v.into_value(),
            None => Value::Null,
        }
    }
}

/// Builds a positional parameter list: `args![a, b, c]` binds `?1`, `?2`, `?3`.
#[macro_export]
macro_rules! args {
    ($($e:expr),* $(,)?) => {
        vec![$($crate::db::Bind::into_value($e)),*] as Vec<libsql::Value>
    };
}

pub fn format_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn format_ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Current time in the stored timestamp format.
pub fn now_ts() -> String {
    format_ts(Utc::now())
}

// ---------------------------------------------------------------- reading rows

/// A result row with by-name typed accessors.
pub struct Record {
    values: Vec<Value>,
    columns: std::sync::Arc<HashMap<String, usize>>,
}

impl Record {
    fn value(&self, name: &str) -> AppResult<&Value> {
        self.columns
            .get(name)
            .and_then(|&i| self.values.get(i))
            .ok_or_else(|| AppError::Internal(format!("column `{name}` missing from result")))
    }

    fn type_error(name: &str, expected: &str, v: &Value) -> AppError {
        AppError::Internal(format!("column `{name}`: expected {expected}, got {v:?}"))
    }

    pub fn opt_text(&self, name: &str) -> AppResult<Option<String>> {
        match self.value(name)? {
            Value::Null => Ok(None),
            Value::Text(s) => Ok(Some(s.clone())),
            v => Err(Self::type_error(name, "text", v)),
        }
    }

    pub fn text(&self, name: &str) -> AppResult<String> {
        self.opt_text(name)?
            .ok_or_else(|| AppError::Internal(format!("column `{name}` is NULL")))
    }

    pub fn opt_i64(&self, name: &str) -> AppResult<Option<i64>> {
        match self.value(name)? {
            Value::Null => Ok(None),
            Value::Integer(i) => Ok(Some(*i)),
            v => Err(Self::type_error(name, "integer", v)),
        }
    }

    pub fn i64(&self, name: &str) -> AppResult<i64> {
        self.opt_i64(name)?
            .ok_or_else(|| AppError::Internal(format!("column `{name}` is NULL")))
    }

    pub fn bool(&self, name: &str) -> AppResult<bool> {
        Ok(self.i64(name)? != 0)
    }

    pub fn uuid(&self, name: &str) -> AppResult<Uuid> {
        let s = self.text(name)?;
        Uuid::parse_str(&s).map_err(|e| AppError::Internal(format!("column `{name}`: {e}")))
    }

    pub fn date(&self, name: &str) -> AppResult<NaiveDate> {
        let s = self.text(name)?;
        NaiveDate::parse_from_str(&s, "%Y-%m-%d")
            .map_err(|e| AppError::Internal(format!("column `{name}`: {e}")))
    }

    pub fn ts(&self, name: &str) -> AppResult<DateTime<Utc>> {
        let s = self.text(name)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|e| AppError::Internal(format!("column `{name}`: {e}")))
    }

    /// The raw value at a position (used by the backup exporter).
    pub fn raw(&self, idx: usize) -> Option<&Value> {
        self.values.get(idx)
    }
}

/// Maps a [`Record`] into a model struct.
pub trait FromRecord: Sized {
    fn from_record(r: &Record) -> AppResult<Self>;
}

/// Runs a query and returns every row as a [`Record`].
pub async fn query_records(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> AppResult<Vec<Record>> {
    let mut rows = conn.query(sql, params).await?;
    let mut columns: Option<std::sync::Arc<HashMap<String, usize>>> = None;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let cols = columns.get_or_insert_with(|| std::sync::Arc::new(column_map(&row)));
        out.push(Record {
            values: row_values(&row)?,
            columns: cols.clone(),
        });
    }
    Ok(out)
}

fn column_map(row: &Row) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for i in 0..row.column_count() {
        if let Some(name) = row.column_name(i) {
            // First occurrence wins (matches `SELECT p.*, …` semantics).
            map.entry(name.to_string()).or_insert(i as usize);
        }
    }
    map
}

fn row_values(row: &Row) -> AppResult<Vec<Value>> {
    (0..row.column_count())
        .map(|i| row.get_value(i).map_err(AppError::from))
        .collect()
}

pub async fn query_all<T: FromRecord>(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> AppResult<Vec<T>> {
    query_records(conn, sql, params)
        .await?
        .iter()
        .map(T::from_record)
        .collect()
}

pub async fn query_opt<T: FromRecord>(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> AppResult<Option<T>> {
    match query_records(conn, sql, params).await?.first() {
        Some(r) => Ok(Some(T::from_record(r)?)),
        None => Ok(None),
    }
}

pub async fn query_one<T: FromRecord>(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> AppResult<T> {
    query_opt(conn, sql, params)
        .await?
        .ok_or_else(|| AppError::NotFound("Not found".into()))
}

/// First column of the first row, or `None` when there are no rows.
pub async fn query_value(
    conn: &Connection,
    sql: &str,
    params: Vec<Value>,
) -> AppResult<Option<Value>> {
    let mut rows = conn.query(sql, params).await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get_value(0)?)),
        None => Ok(None),
    }
}

/// A single integer (e.g. `COUNT(*)`, `COALESCE(SUM(...), 0)`). NULL and no rows are 0.
pub async fn query_i64(conn: &Connection, sql: &str, params: Vec<Value>) -> AppResult<i64> {
    match query_value(conn, sql, params).await? {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Integer(i)) => Ok(i),
        Some(v) => Err(AppError::Internal(format!(
            "expected an integer, got {v:?}"
        ))),
    }
}

/// Executes a statement and returns the number of affected rows.
pub async fn execute(conn: &Connection, sql: &str, params: Vec<Value>) -> AppResult<u64> {
    Ok(conn.execute(sql, params).await?)
}

/// Retry policy used by [`Db::open_with_retry`] at startup: Turso Cloud or the network may be
/// briefly unavailable when a container starts.
pub async fn open_with_retry(target: &DbTarget, attempts: u32) -> AppResult<Db> {
    let mut last_err = None;
    for attempt in 1..=attempts.max(1) {
        let result = async {
            let db = Db::open(target).await?;
            db.ping().await?;
            Ok::<_, AppError>(db)
        }
        .await;
        match result {
            Ok(db) => return Ok(db),
            Err(e) => {
                tracing::warn!(
                    "database not reachable (attempt {attempt}/{attempts}): {}",
                    e.detail()
                );
                last_err = Some(e);
                if attempt < attempts {
                    tokio::time::sleep(Duration::from_secs(u64::from(attempt.min(5)))).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::Internal("database unavailable".into())))
}
