//! PostgreSQL access through SQLx.
//!
//! - [`Db`] wraps a connection pool (`PgPool`). Handlers take one pooled connection per request
//!   with [`Db::conn`] and pass `&mut conn` (or `&mut tx`) to the query helpers below.
//! - SQL uses PostgreSQL placeholders (`$1`, `$2`, …; a number may repeat) and every value is
//!   bound through [`args!`](crate::args), never formatted into the SQL string.
//! - Rows are read by column name through [`Record`] and mapped with [`FromRecord`].

use std::{collections::HashMap, fmt, str::FromStr, sync::Arc, time::Duration};

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{
    pool::PoolConnection,
    postgres::{PgArguments, PgConnectOptions, PgPoolOptions, PgRow},
    Arguments, Column, Decode, Encode, PgPool, Postgres, Row, Transaction, Type,
};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// A single PostgreSQL connection (pooled connections and transactions deref to it).
pub type Connection = sqlx::PgConnection;

// ---------------------------------------------------------------- settings

/// Connection settings, read from `DATABASE_URL` and the `DATABASE_*` tuning variables.
///
/// `Debug` and [`DbSettings::describe`] never include the password.
#[derive(Clone)]
pub struct DbSettings {
    url: String,
    /// Connect to this database instead of the one in the URL (same server and credentials).
    database_override: Option<String>,
    /// Upper bound of pooled connections per backend instance.
    pub max_connections: u32,
    /// How long a request waits for a free connection before failing with 503.
    pub acquire_timeout: Duration,
    /// Server-side limit for a single statement (`statement_timeout`).
    pub statement_timeout: Duration,
}

impl fmt::Debug for DbSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbSettings")
            .field("database", &self.describe())
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .field("statement_timeout", &self.statement_timeout)
            .finish()
    }
}

const URL_HELP: &str =
    "DATABASE_URL must be a PostgreSQL URL like postgres://user:password@host:5432/dbname";

impl DbSettings {
    pub fn new(url: &str) -> Result<Self, String> {
        let url = url.trim();
        if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
            return Err(URL_HELP.into());
        }
        // Validate now so a typo fails at startup with a clear message (without echoing the URL,
        // which contains the password).
        PgConnectOptions::from_str(url).map_err(|_| URL_HELP.to_string())?;
        Ok(Self {
            url: url.to_string(),
            database_override: None,
            max_connections: 10,
            acquire_timeout: Duration::from_secs(5),
            statement_timeout: Duration::from_secs(30),
        })
    }

    /// The same server and credentials, but another database (used by the test harness).
    pub fn with_database(&self, name: &str) -> Self {
        Self {
            database_override: Some(name.to_string()),
            ..self.clone()
        }
    }

    /// Where these settings connect to (no credentials).
    pub fn destination(&self) -> Result<Destination, String> {
        let mut d = destination(&self.url)?;
        if let Some(name) = &self.database_override {
            d.database = name.clone();
        }
        Ok(d)
    }

    pub fn connect_options(&self) -> AppResult<PgConnectOptions> {
        let mut opts = PgConnectOptions::from_str(&self.url)
            .map_err(|_| AppError::Internal(URL_HELP.into()))?;
        if let Some(name) = &self.database_override {
            opts = opts.database(name);
        }
        Ok(opts.application_name("hourwise-backend").options([(
            "statement_timeout",
            format!("{}ms", self.statement_timeout.as_millis()),
        )]))
    }

    /// `PostgreSQL user@host:port/db`, safe to log.
    pub fn describe(&self) -> String {
        match self.destination() {
            Ok(d) => format!("PostgreSQL {d}"),
            Err(_) => "PostgreSQL (unparseable URL)".into(),
        }
    }
}

// ---------------------------------------------------------------- pool

/// Shared handle to the connection pool. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    /// Opens the pool and establishes one connection (so bad credentials fail immediately).
    pub async fn connect(settings: &DbSettings) -> AppResult<Self> {
        let pool = pool_options(settings)
            .connect_with(settings.connect_options()?)
            .await?;
        Ok(Self { pool })
    }

    /// A pool that connects on first use (used by tests that simulate an outage).
    pub fn connect_lazy(settings: &DbSettings) -> AppResult<Self> {
        let pool = pool_options(settings).connect_lazy_with(settings.connect_options()?);
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Takes a connection from the pool (returned automatically when dropped).
    pub async fn conn(&self) -> AppResult<PoolConnection<Postgres>> {
        Ok(self.pool.acquire().await?)
    }

    /// Checks that the database is reachable and the credentials are valid.
    pub async fn ping(&self) -> AppResult<()> {
        let mut conn = self.conn().await?;
        let one = query_i64(&mut conn, "SELECT 1", crate::args![]).await?;
        if one != 1 {
            return Err(AppError::Internal("unexpected ping result".into()));
        }
        Ok(())
    }

    /// Closes every pooled connection (waits for in-flight queries).
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

fn pool_options(settings: &DbSettings) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(settings.max_connections.max(1))
        .min_connections(0)
        .acquire_timeout(settings.acquire_timeout)
        // Recycle idle and old connections so restarts/failovers of PostgreSQL heal by themselves.
        .idle_timeout(Some(Duration::from_secs(10 * 60)))
        .max_lifetime(Some(Duration::from_secs(30 * 60)))
        .test_before_acquire(true)
}

/// Connects with retries: at container start PostgreSQL may still be booting.
pub async fn connect_with_retry(settings: &DbSettings, attempts: u32) -> AppResult<Db> {
    let mut last_err = None;
    for attempt in 1..=attempts.max(1) {
        let result = async {
            let db = Db::connect(settings).await?;
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

/// Starts a transaction. Everything runs atomically and is rolled back if the transaction is
/// dropped without [`Transaction::commit`] (e.g. when a handler returns early with `?`).
///
/// For read-check-write rules (like the 24h daily cap), call [`lock_user`] first so concurrent
/// requests for the same user are serialised.
pub async fn write_tx(conn: &mut Connection) -> AppResult<Transaction<'_, Postgres>> {
    Ok(sqlx::Connection::begin(conn).await?)
}

/// Locks a user's row until the surrounding transaction ends (`SELECT … FOR UPDATE`). Requests
/// for *other* users are not blocked.
pub async fn lock_user(conn: &mut Connection, user_id: Uuid) -> AppResult<()> {
    query_records(
        conn,
        "SELECT id FROM users WHERE id = $1 FOR UPDATE",
        crate::args![user_id],
    )
    .await?;
    Ok(())
}

/// Current time as stored by PostgreSQL (`TIMESTAMPTZ` has microsecond precision), so values
/// returned to clients match what a later read returns.
pub fn now_ts() -> DateTime<Utc> {
    let now = Utc::now();
    DateTime::from_timestamp_micros(now.timestamp_micros()).unwrap_or(now)
}

// ---------------------------------------------------------------- binding values

/// Positional parameters for one statement. Build it with [`args!`](crate::args).
#[derive(Default)]
pub struct Args {
    inner: PgArguments,
    error: Option<String>,
}

impl Args {
    pub fn push<'q, T>(&mut self, value: T)
    where
        T: 'q + Encode<'q, Postgres> + Type<Postgres>,
    {
        if let Err(e) = <PgArguments as Arguments<'q>>::add(&mut self.inner, value) {
            self.error.get_or_insert_with(|| e.to_string());
        }
    }

    fn into_inner(self) -> AppResult<PgArguments> {
        match self.error {
            Some(e) => Err(AppError::Internal(format!("cannot bind parameter: {e}"))),
            None => Ok(self.inner),
        }
    }
}

/// Builds a positional parameter list: `args![a, b, c]` binds `$1`, `$2`, `$3`.
#[macro_export]
macro_rules! args {
    ($($e:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut a = $crate::db::Args::default();
        $( a.push($e); )*
        a
    }};
}

// ---------------------------------------------------------------- reading rows

/// A result row with by-name typed accessors.
pub struct Record {
    row: PgRow,
    columns: Arc<HashMap<String, usize>>,
}

impl Record {
    fn index(&self, name: &str) -> AppResult<usize> {
        self.columns
            .get(name)
            .copied()
            .ok_or_else(|| AppError::Internal(format!("column `{name}` missing from result")))
    }

    fn get<'r, T>(&'r self, name: &str) -> AppResult<T>
    where
        T: Decode<'r, Postgres> + Type<Postgres>,
    {
        let i = self.index(name)?;
        self.row
            .try_get::<T, _>(i)
            .map_err(|e| AppError::Internal(format!("column `{name}`: {e}")))
    }

    pub fn opt_text(&self, name: &str) -> AppResult<Option<String>> {
        self.get(name)
    }

    pub fn text(&self, name: &str) -> AppResult<String> {
        self.get(name)
    }

    fn opt_i64_at(&self, i: usize) -> AppResult<Option<i64>> {
        // COUNT/SUM return BIGINT, INTEGER columns return INT4: accept both.
        match self.row.try_get::<Option<i64>, _>(i) {
            Ok(v) => Ok(v),
            Err(_) => self
                .row
                .try_get::<Option<i32>, _>(i)
                .map(|v| v.map(i64::from))
                .map_err(|e| AppError::Internal(format!("column {i}: expected an integer: {e}"))),
        }
    }

    pub fn opt_i64(&self, name: &str) -> AppResult<Option<i64>> {
        self.opt_i64_at(self.index(name)?)
    }

    pub fn i64(&self, name: &str) -> AppResult<i64> {
        self.opt_i64(name)?
            .ok_or_else(|| AppError::Internal(format!("column `{name}` is NULL")))
    }

    pub fn bool(&self, name: &str) -> AppResult<bool> {
        self.get(name)
    }

    pub fn uuid(&self, name: &str) -> AppResult<Uuid> {
        self.get(name)
    }

    pub fn date(&self, name: &str) -> AppResult<NaiveDate> {
        self.get(name)
    }

    pub fn ts(&self, name: &str) -> AppResult<DateTime<Utc>> {
        self.get(name)
    }
}

/// Maps a [`Record`] into a model struct.
pub trait FromRecord: Sized {
    fn from_record(r: &Record) -> AppResult<Self>;
}

fn column_map(row: &PgRow) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for col in row.columns() {
        // First occurrence wins (matches `SELECT p.*, …` semantics).
        map.entry(col.name().to_string()).or_insert(col.ordinal());
    }
    map
}

/// Runs a query and returns every row as a [`Record`].
pub async fn query_records(conn: &mut Connection, sql: &str, args: Args) -> AppResult<Vec<Record>> {
    let rows = sqlx::query_with(sql, args.into_inner()?)
        .fetch_all(conn)
        .await?;
    let Some(first) = rows.first() else {
        return Ok(Vec::new());
    };
    let columns = Arc::new(column_map(first));
    Ok(rows
        .into_iter()
        .map(|row| Record {
            row,
            columns: columns.clone(),
        })
        .collect())
}

pub async fn query_all<T: FromRecord>(
    conn: &mut Connection,
    sql: &str,
    args: Args,
) -> AppResult<Vec<T>> {
    query_records(conn, sql, args)
        .await?
        .iter()
        .map(T::from_record)
        .collect()
}

pub async fn query_opt<T: FromRecord>(
    conn: &mut Connection,
    sql: &str,
    args: Args,
) -> AppResult<Option<T>> {
    match query_records(conn, sql, args).await?.first() {
        Some(r) => Ok(Some(T::from_record(r)?)),
        None => Ok(None),
    }
}

pub async fn query_one<T: FromRecord>(
    conn: &mut Connection,
    sql: &str,
    args: Args,
) -> AppResult<T> {
    query_opt(conn, sql, args)
        .await?
        .ok_or_else(|| AppError::NotFound("Not found".into()))
}

/// First column of the first row, or `None` when there are no rows.
pub async fn query_scalar<T>(conn: &mut Connection, sql: &str, args: Args) -> AppResult<Option<T>>
where
    T: for<'r> Decode<'r, Postgres> + Type<Postgres>,
{
    let row = sqlx::query_with(sql, args.into_inner()?)
        .fetch_optional(conn)
        .await?;
    match row {
        None => Ok(None),
        Some(row) => row
            .try_get::<T, _>(0)
            .map(Some)
            .map_err(|e| AppError::Internal(format!("column 0: {e}"))),
    }
}

/// A single integer (e.g. `COUNT(*)`, `COALESCE(SUM(...), 0)`). NULL and no rows are 0.
pub async fn query_i64(conn: &mut Connection, sql: &str, args: Args) -> AppResult<i64> {
    match query_records(conn, sql, args).await?.first() {
        Some(r) => Ok(r.opt_i64_at(0)?.unwrap_or(0)),
        None => Ok(0),
    }
}

/// Executes a statement and returns the number of affected rows.
pub async fn execute(conn: &mut Connection, sql: &str, args: Args) -> AppResult<u64> {
    Ok(sqlx::query_with(sql, args.into_inner()?)
        .execute(conn)
        .await?
        .rows_affected())
}

// ---------------------------------------------------------------- destinations

/// Which database a URL points at, normalised so that different spellings of the same
/// destination compare equal (`postgres://` vs `postgresql://`, host case, `localhost` vs
/// `127.0.0.1` vs `::1`, an omitted default port or database name, percent-encoding, query
/// parameters). Equality ignores the user name and password: the same database reached with
/// different credentials is still the same database. `Display` never shows the password.
#[derive(Debug, Clone)]
pub struct Destination {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
}

impl PartialEq for Destination {
    fn eq(&self, other: &Self) -> bool {
        self.same_server(other) && self.database == other.database
    }
}
impl Eq for Destination {}

impl Destination {
    /// Same PostgreSQL server (host and port), regardless of the database name.
    pub fn same_server(&self, other: &Destination) -> bool {
        self.host == other.host && self.port == other.port
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{}:{}/{}",
            self.user, self.host, self.port, self.database
        )
    }
}

/// Hostnames that all mean "this machine" (including Docker's alias for the host).
fn normalise_host(host: &str) -> String {
    let h = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    match h.as_str() {
        ""
        | "localhost"
        | "127.0.0.1"
        | "::1"
        | "0:0:0:0:0:0:0:1"
        | "0.0.0.0"
        | "host.docker.internal"
        | "host-gateway" => "localhost".into(),
        _ if h.starts_with("127.") => "localhost".into(),
        _ => h,
    }
}

/// Parses a PostgreSQL URL into a [`Destination`]. The error never echoes the URL.
pub fn destination(url: &str) -> Result<Destination, String> {
    let url = url.trim();
    if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
        return Err("not a postgres:// URL".into());
    }
    let opts = PgConnectOptions::from_str(url).map_err(|_| "invalid PostgreSQL URL".to_string())?;
    let user = opts.get_username().to_string();
    // PostgreSQL connects to a database named after the user when none is given.
    let database = opts
        .get_database()
        .map_or_else(|| user.clone(), str::to_string);
    Ok(Destination {
        host: normalise_host(opts.get_host()),
        port: opts.get_port(),
        database,
        user,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dest(url: &str) -> Destination {
        destination(url).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn equivalent_urls_normalise_to_the_same_destination() {
        let base = dest("postgres://app:secret@localhost:5432/hourwise");
        for url in [
            "postgresql://app:other@127.0.0.1/hourwise",
            "postgres://app@LOCALHOST:5432/hourwise?sslmode=disable",
            "postgres://app@[::1]:5432/hourwise",
            "postgres://app@host.docker.internal:5432/hourwise",
            "postgres://app@localhost:5432/%68ourwise",
            "  postgres://app@localhost/hourwise  ",
        ] {
            assert_eq!(dest(url), base, "{url}");
        }
        // Omitted database name means "same as the user".
        assert_eq!(dest("postgres://hourwise@db.internal").database, "hourwise");
        // A different user (or password) for the same database is still the same database.
        assert_eq!(dest("postgres://admin:x@localhost/hourwise"), base);
        assert_ne!(dest("postgres://app@localhost:5433/hourwise"), base);
        assert_ne!(dest("postgres://app@localhost/other"), base);
        assert!(dest("postgres://a@localhost:5432/x").same_server(&base));
        assert!(!dest("postgres://a@localhost:55432/x").same_server(&base));
        assert!(!dest("postgres://a@db.example.com/hourwise").same_server(&base));
    }

    #[test]
    fn passwords_never_appear_in_descriptions() {
        let s = DbSettings::new("postgres://app:hunter2-secret@db:5432/hourwise")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!s.describe().contains("hunter2"));
        assert!(!format!("{s:?}").contains("hunter2"));
        assert_eq!(s.describe(), "PostgreSQL app@db:5432/hourwise");
        let err = DbSettings::new("mysql://app:hunter2@db/x")
            .err()
            .unwrap_or_default();
        assert!(!err.contains("hunter2"));
        assert!(destination("libsql://x.turso.io").is_err());
    }
}
