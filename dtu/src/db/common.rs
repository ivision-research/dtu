use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::result;
use std::sync::{Arc, Mutex, RwLock};

use diesel::connection::SimpleConnection;
use diesel::migration::MigrationSource;
use diesel::r2d2::{
    self, ConnectionManager, CustomizeConnection, Pool, PoolError, PooledConnection,
};
use diesel::result::{DatabaseErrorInformation, DatabaseErrorKind, Error as DieselError};
use diesel::sqlite::Sqlite;
use diesel::{prelude::*, sql_query};
use diesel::{ConnectionError, SqliteConnection};
use diesel_migrations::MigrationHarness;
use lazy_static::lazy_static;

use crate::db::query;
use crate::utils::ensure_dir_exists;
use crate::Context;
use dtu_proc_macro::wraps_base_error;

pub const FRAMEWORK_SOURCE: &'static str = "framework";

#[derive(Debug)]
pub struct DBErrorInfo {
    pub message: String,
    pub details: Option<String>,
    pub hint: Option<String>,
}

impl From<Box<dyn DatabaseErrorInformation + Send + Sync>> for DBErrorInfo {
    fn from(value: Box<dyn DatabaseErrorInformation + Send + Sync>) -> Self {
        let message = String::from(value.message());
        let details = value.details().map(|it| it.to_string());
        let hint = value.hint().map(|it| it.to_string());
        Self {
            message,
            details,
            hint,
        }
    }
}

impl Display for DBErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(details) = self.details.as_ref() {
            write!(f, "\nDetails:\n{}", details)?;
        }
        if let Some(hint) = self.hint.as_ref() {
            write!(f, "\nHint:\n{}", hint)?;
        }
        Ok(())
    }
}

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("connection error: {0}")]
    ConnectionError(ConnectionError),
    #[error("requested database entry not found")]
    NotFound,
    #[error("invalid query")]
    InvalidQuery,
    #[error("database error {0:?}: {1}")]
    DatabaseError(DatabaseErrorKind, DBErrorInfo),
    #[error("{0}")]
    UniqueViolation(DBErrorInfo),
    #[error("{0}")]
    ForeignKeyViolation(DBErrorInfo),
    #[error("{0}")]
    NonNullViolation(DBErrorInfo),
    #[error("connection pool error: {0}")]
    Pool(PoolError),
    #[error("attempted a write inside another write on the same database")]
    ReentrantWrite,
    #[error("generic database error: {0}")]
    Generic(String),
}

impl From<PoolError> for Error {
    fn from(value: PoolError) -> Self {
        Self::Pool(value)
    }
}

impl From<DieselError> for Error {
    fn from(value: DieselError) -> Self {
        match value {
            DieselError::InvalidCString(_) => Self::InvalidQuery,
            DieselError::NotFound => Self::NotFound,
            DieselError::QueryBuilderError(e) => Self::Generic(e.to_string()),
            DieselError::DeserializationError(e) => Self::Generic(e.to_string()),
            DieselError::SerializationError(e) => Self::Generic(e.to_string()),
            DieselError::DatabaseError(kind, info) => match kind {
                DatabaseErrorKind::NotNullViolation => Self::NonNullViolation(info.into()),
                DatabaseErrorKind::ForeignKeyViolation => Self::ForeignKeyViolation(info.into()),
                DatabaseErrorKind::UniqueViolation => Self::UniqueViolation(info.into()),
                _ => Self::DatabaseError(kind, info.into())
            },
            DieselError::RollbackErrorOnCommit { .. } => Self::Generic("rollback error".into()),
            DieselError::AlreadyInTransaction => Self::Generic("attempted to perform an illegal operation inside of a transaction".into()),
            DieselError::NotInTransaction => Self::Generic("attempted to perform an operation outside of a transaction that requires a transaction".into()),
            DieselError::RollbackTransaction => Self::Generic("unexpected transaction error".into()),
            DieselError::BrokenTransactionManager => Self::Generic("transaction manager broken, likely due to a broken connection".into()),
            _ => Self::Generic(format!("unexpected error {:?}", value)),
        }
    }
}

pub type Result<T> = result::Result<T, Error>;

/// Every pragma any part of the crate changes must appear here at its baseline value, since
/// this is also what [Db::write_with_pragmas] restores afterwards. That is why you'll see a lot of
/// SQLite defaults in here.
const CONNECTION_PRAGMAS: &str = "\
PRAGMA journal_mode = DELETE;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = FULL;
PRAGMA temp_store = DEFAULT;";

/// Apply [CONNECTION_PRAGMAS] to a connection
///
/// Call this first from a custom [Customizer] so that a pool needing extra setup, an ATTACH
/// for instance, still starts from the state every other pool is in.
pub(crate) fn apply_connection_pragmas(
    conn: &mut SqliteConnection,
) -> result::Result<(), r2d2::Error> {
    conn.batch_execute(CONNECTION_PRAGMAS)
        .map_err(r2d2::Error::QueryError)
}

/// Per connection setup, run once as the pool opens each connection
///
/// Note that r2d2 customises a connection when it is created, not on every checkout, so
/// anything done here lasts for the life of that connection.
pub(crate) type Customizer = Box<dyn CustomizeConnection<SqliteConnection, r2d2::Error>>;

/// The default [Customizer], which does nothing but apply the baseline pragmas
#[derive(Debug)]
struct SqlitePragmas;

impl CustomizeConnection<SqliteConnection, r2d2::Error> for SqlitePragmas {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> result::Result<(), r2d2::Error> {
        apply_connection_pragmas(conn)
    }
}

type PooledSqlite = PooledConnection<ConnectionManager<SqliteConnection>>;

/// A pool of connections to a single sqlite database
///
/// Cloning is cheap: all clones of a [Db] for the same file share the pool and the write
/// lock.
#[derive(Clone)]
pub struct Db(Arc<DbInner>);

struct DbInner {
    pool: Pool<ConnectionManager<SqliteConnection>>,
    /// Serialises writers so only one connection is in a write transaction at a time
    write_lock: Mutex<()>,
}

impl Db {
    pub fn check_fts5(&self) -> bool {
        #[derive(QueryableByName)]
        struct CompileOption {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            enabled: i32,
        }
        self.query(|c| {
            query!(sql_query(
                "SELECT sqlite_compileoption_used('ENABLE_FTS5') AS enabled"
            ))
            .get_result::<CompileOption>(c)
        })
        .is_ok_and(|it| it.enabled == 1)
    }

    /// Read using any available connection
    ///
    /// Runs concurrently with other reads and with a write.
    pub fn query<F, R, E>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut SqliteConnection) -> result::Result<R, E>,
        E: Into<Error>,
    {
        let mut conn = self.get_connection()?;
        f(&mut conn).map_err(Into::into)
    }

    /// Write, holding one connection for the whole closure inside a transaction
    ///
    /// Serialised against other writers. Because the connection is held, temporary tables
    /// and per-connection state stay valid for the duration.
    ///
    /// Calling this from inside itself would deadlock, so it fails with
    /// [Error::ReentrantWrite] instead. A [Db::query] inside the closure is allowed but
    /// will not see the uncommitted writes.
    pub fn write<F, R, E>(&self, f: F) -> result::Result<R, E>
    where
        F: FnOnce(&mut SqliteConnection) -> result::Result<R, E>,
        E: From<Error> + From<DieselError>,
    {
        self.write_with_pragmas("", f)
    }

    /// [Db::write], with `pragmas` applied to the connection before the transaction opens
    ///
    /// For settings a transaction cannot change, `foreign_keys` in particular, which is a
    /// silent no-op inside one.
    ///
    /// The connection is put back to [CONNECTION_PRAGMAS] on the way out, however this
    /// returns. r2d2 customises a connection only when it opens it, so without that reset
    /// the change would outlive this call on whichever connection happened to serve it.
    pub fn write_with_pragmas<F, R, E>(&self, pragmas: &str, f: F) -> result::Result<R, E>
    where
        F: FnOnce(&mut SqliteConnection) -> result::Result<R, E>,
        E: From<Error> + From<DieselError>,
    {
        let _reentry = ReentryGuard::acquire(self.key()).map_err(E::from)?;
        let _writing = self.0.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = self.get_connection().map_err(E::from)?;

        if pragmas.is_empty() {
            let mut conn = conn;
            let conn: &mut SqliteConnection = &mut conn;
            return conn.transaction(f);
        }

        let mut guard = PragmaReset(conn);
        let conn: &mut SqliteConnection = &mut guard.0;
        conn.batch_execute(pragmas).map_err(DieselError::from)?;
        conn.transaction(f)
    }

    fn get_connection(&self) -> Result<PooledSqlite> {
        self.0.pool.get().map_err(Error::from)
    }

    /// Identifies the database this handle points at, shared by all clones
    fn key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

thread_local! {
    /// The databases this thread is currently inside a [Db::write] on
    static ACTIVE_WRITES: RefCell<Vec<usize>> = RefCell::new(Vec::new());
}

/// Returns a connection to [CONNECTION_PRAGMAS] when the write that changed them ends
///
/// A guard rather than a statement at the end of the write so that a panic inside the
/// closure cannot put a connection back in the pool with, say, foreign keys still off.
struct PragmaReset(PooledSqlite);

impl Drop for PragmaReset {
    fn drop(&mut self) {
        if let Err(e) = self.0.batch_execute(CONNECTION_PRAGMAS) {
            // Nothing here can evict the connection from the pool, so all this can do is
            // say so as loudly as it can
            log::error!("failed to restore the connection pragmas: {}", e);
        }
    }
}

/// Detects a [Db::write] entered while this thread is already inside one on the same
/// database, which would otherwise deadlock on the write lock with no output
struct ReentryGuard(usize);

impl ReentryGuard {
    fn acquire(key: usize) -> Result<Self> {
        ACTIVE_WRITES.with(|active| {
            let mut active = active.borrow_mut();
            if active.contains(&key) {
                return Err(Error::ReentrantWrite);
            }
            active.push(key);
            Ok(Self(key))
        })
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        ACTIVE_WRITES.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(idx) = active.iter().rposition(|it| *it == self.0) {
                active.swap_remove(idx);
            }
        });
    }
}

// One pool per database file, so that the write lock covers every writer to that file and
// migrations run exactly once no matter how many handles are created.
lazy_static! {
    static ref DATABASES: RwLock<HashMap<String, Db>> = RwLock::new(HashMap::new());
}

impl Db {
    pub fn new(
        ctx: &dyn Context,
        file_name: &str,
        migrations: impl MigrationSource<Sqlite>,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite>,
    ) -> Result<Self> {
        let mut path = ctx.get_sqlite_dir()?;
        ensure_dir_exists(&path)?;
        path.push(file_name);
        let url = format!("sqlite://{}", path.to_string_lossy());
        Self::new_from_url(
            &url,
            migrations,
            #[cfg(test)]
            test_migrations,
        )
    }

    pub fn new_from_path<S: AsRef<str> + ?Sized>(
        path: &S,
        migrations: impl MigrationSource<Sqlite>,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite>,
    ) -> Result<Self> {
        Self::new_from_path_with(
            path,
            migrations,
            #[cfg(test)]
            test_migrations,
            None,
        )
    }

    /// [Db::new_from_path] with per connection setup beyond the baseline pragmas
    ///
    /// The customizer is only used if this is the first handle to the file: pools are shared
    /// per database, so a later call for a file that is already open gets the existing pool
    /// and its original customizer.
    pub fn new_from_path_with<S: AsRef<str> + ?Sized>(
        path: &S,
        migrations: impl MigrationSource<Sqlite>,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite>,
        customizer: Option<Customizer>,
    ) -> Result<Self> {
        let url = format!("sqlite://{}", path.as_ref());
        Self::new_from_url_with(
            &url,
            migrations,
            #[cfg(test)]
            test_migrations,
            customizer,
        )
    }

    pub fn new_from_url(
        url: &String,
        migrations: impl MigrationSource<Sqlite>,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite>,
    ) -> Result<Self> {
        Self::new_from_url_with(
            url,
            migrations,
            #[cfg(test)]
            test_migrations,
            None,
        )
    }

    pub fn new_from_url_with(
        url: &String,
        migrations: impl MigrationSource<Sqlite>,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite>,
        customizer: Option<Customizer>,
    ) -> Result<Self> {
        if let Some(db) = DATABASES.read().unwrap().get(url) {
            return Ok(db.clone());
        }

        let mut map = DATABASES.write().unwrap();
        // Another thread may have created it between dropping the read lock and taking
        // the write lock
        if let Some(db) = map.get(url) {
            return Ok(db.clone());
        }

        let db = Self::connect(
            url,
            migrations,
            #[cfg(test)]
            test_migrations,
            customizer.unwrap_or_else(|| Box::new(SqlitePragmas)),
        )?;
        map.insert(url.clone(), db.clone());
        Ok(db)
    }

    fn connect(
        url: &String,
        migrations: impl MigrationSource<Sqlite>,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite>,
        customizer: Customizer,
    ) -> Result<Self> {
        log::debug!("connecting to the database at {}", url);

        let pool = Pool::builder()
            .max_size(max_connections())
            // Connections are opened on demand, not all at once up front
            .min_idle(Some(1))
            // Same as the r2d2 default, stated here because a starved pool surfaces as a
            // checkout error after this long rather than as a hang
            .connection_timeout(std::time::Duration::from_secs(30))
            .connection_customizer(customizer)
            .build(ConnectionManager::<SqliteConnection>::new(url))
            .map_err(Error::from)?;

        // Migrations run once, before the pool serves anyone else. Several connections
        // running them on one file concurrently is not safe.
        let mut conn = pool.get().map_err(Error::from)?;
        conn.run_pending_migrations(migrations)?;
        #[cfg(test)]
        conn.run_pending_migrations(test_migrations)
            .expect("failed to load test migrations");
        drop(conn);

        Ok(Self(Arc::new(DbInner {
            pool,
            write_lock: Mutex::new(()),
        })))
    }
}

/// Drop the pool for `url`, closing its connections
///
/// A later [Db::new_from_url] for the same file creates a fresh pool.
#[allow(dead_code)]
pub(crate) fn cleanup_database(url: &String) {
    DATABASES.write().unwrap().remove(url);
}

fn max_connections() -> u32 {
    // Two spare: one for a writer and one for whatever opportunistic read the writer
    // itself needs.
    const SPARE: u32 = 2;
    // Used when the parallelism is unavailable or absurd
    const FALLBACK: u32 = 4;

    let parallelism = std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(FALLBACK as usize);
    u32::try_from(parallelism).unwrap_or(FALLBACK) + SPARE
}

/// Trait for all types that are used as database IDs
///
/// Implemented with the [database_id] macro
pub trait DatabaseId:
    Clone
    + Copy
    + PartialEq
    + Eq
    + Hash
    + PartialOrd
    + Ord
    + Debug
    + serde::Serialize
    + serde::de::DeserializeOwned
{
    fn from_id(id: i32) -> Self;
    /// Retrieve the raw database id
    fn id(self) -> i32;
}

impl<T> Idable for T
where
    for<'a> &'a T: Identifiable<Id = &'a i32>,
{
    fn get_id(&self) -> i32 {
        *self.id()
    }
}

pub trait Idable {
    fn get_id(&self) -> i32;
}
