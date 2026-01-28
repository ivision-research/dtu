#![allow(unused_macros)]

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, RwLock};

use diesel::connection::SimpleConnection;
use diesel::migration::MigrationSource;
use diesel::prelude::*;
use diesel::result::{DatabaseErrorInformation, DatabaseErrorKind, Error as DieselError};
use diesel::sqlite::Sqlite;
use diesel::{ConnectionError, SqliteConnection};
use diesel_migrations::MigrationHarness;
use lazy_static::lazy_static;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::utils::{ensure_dir_exists, ClassName};
use crate::Context;
use dtu_proc_macro::wraps_base_error;

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
    #[error("generic database error: {0}")]
    Generic(String),
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

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub(super) struct DBThread(Arc<ThreadPool>);

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

pub trait Enablable {
    fn is_enabled(&self) -> bool;
}

pub trait Exportable {
    fn is_exported(&self) -> bool;
}

pub trait ApkComponent {
    fn get_apk_id(&self) -> i32;
}

#[derive(Clone, Copy)]
pub enum PermissionMode {
    /// Generic permission on the entire object
    Generic,
    /// Separate permission for Read operations
    Read,
    /// Separate permission for Write operations
    Write,
}

pub trait PermissionProtected {
    /// Return true if at least one permission mode is required
    fn requires_permission(&self) -> bool {
        self.get_permission_for_mode(PermissionMode::Generic)
            .is_some()
            || self.get_permission_for_mode(PermissionMode::Read).is_some()
            || self
                .get_permission_for_mode(PermissionMode::Write)
                .is_some()
    }

    /// Gets the permission for the given [PermissionMode]
    fn get_permission_for_mode(&self, mode: PermissionMode) -> Option<&str> {
        match mode {
            PermissionMode::Generic => self.get_generic_permission(),
            _ => None,
        }
    }

    fn get_generic_permission(&self) -> Option<&str>;

    #[deprecated(note = "Use get_generic_permission")]
    fn get_permission(&self) -> Option<&str> {
        self.get_generic_permission()
    }
}

#[derive(Clone, Copy)]
pub enum ApkIPCKind {
    Receiver,
    Activity,
    Provider,
    Service,
}

/// A trait for Receivers, Activities, and Services
pub trait ApkIPC: ApkComponent + Exportable + PermissionProtected + Enablable + Idable {
    fn get_class_name(&self) -> ClassName;
    fn get_package(&self) -> Cow<'_, str>;
    fn get_kind(&self) -> ApkIPCKind;
}

impl DBThread {
    pub(super) fn new(
        ctx: &dyn Context,
        file_name: &str,
        migrations: impl MigrationSource<Sqlite> + Send,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite> + Send,
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

    pub(super) fn new_from_path<S: AsRef<str> + ?Sized>(
        path: &S,
        migrations: impl MigrationSource<Sqlite> + Send,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite> + Send,
    ) -> Result<Self> {
        let url = format!("sqlite://{}", path.as_ref());
        Self::new_from_url(
            &url,
            migrations,
            #[cfg(test)]
            test_migrations,
        )
    }

    pub(super) fn new_from_url(
        url: &String,
        migrations: impl MigrationSource<Sqlite> + Send,
        #[cfg(test)] test_migrations: impl MigrationSource<Sqlite> + Send,
    ) -> Result<Self> {
        let db_thread = get_database_threadpool(
            url,
            migrations,
            #[cfg(test)]
            test_migrations,
        )?;
        Ok(Self(db_thread))
    }

    pub(super) fn transaction<F, T, E>(&self, f: F) -> std::result::Result<T, E>
    where
        T: Send,
        E: From<diesel::result::Error> + Send,
        F: FnOnce(&mut SqliteConnection) -> std::result::Result<T, E> + Send,
    {
        self.0.install(|| {
            CONNECTION.with(|c| {
                let mut borrowed = c.borrow_mut();
                let conn = borrowed.as_mut().unwrap();
                conn.transaction(f)
            })
        })
    }

    pub(super) fn with_connection<F, R>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce(&mut SqliteConnection) -> R + Send,
    {
        self.0.install(|| {
            CONNECTION.with(|c| {
                let mut borrowed = c.borrow_mut();
                let conn = borrowed.as_mut().unwrap();

                f(conn)
            })
        })
    }
}

// To be a bit lazy with the design here, we're going to maintain a global
// map of database URLs -> single threaded thread pool. Then, when a new
// database is opened, we'll create a thread local connection to the database
// in that thread pool's thread. Then every database operation will happen
// via calls to that connection in that single thread
//
// This is basically a memory leak, but whatever
lazy_static! {
    static ref DB_THREADS: RwLock<HashMap<String, Arc<ThreadPool>>> = RwLock::new(HashMap::new());
}

thread_local! {
    static CONNECTION: RefCell<Option<SqliteConnection>> = RefCell::new(None);
}

pub(super) fn get_database_threadpool(
    url: &String,
    migrations: impl MigrationSource<Sqlite> + Send,
    #[cfg(test)] test_migrations: impl MigrationSource<Sqlite> + Send,
) -> Result<Arc<ThreadPool>> {
    match try_get_database_threadpool(url) {
        Some(v) => return Ok(v),
        None => {}
    };
    let mut map = DB_THREADS.write().unwrap();
    // Have to check again after getting the write lock. This won't
    // happen often
    if let Some(v) = map.get(url) {
        return Ok(Arc::clone(v));
    }
    // Otherwise we're creating it
    new_database_threadpool(
        url,
        &mut map,
        migrations,
        #[cfg(test)]
        test_migrations,
    )
}

fn try_get_database_threadpool(url: &String) -> Option<Arc<ThreadPool>> {
    let map = DB_THREADS.read().unwrap();
    map.get(url).map(|v| Arc::clone(v))
}

fn new_database_threadpool(
    url: &String,
    map: &mut HashMap<String, Arc<ThreadPool>>,
    migrations: impl MigrationSource<Sqlite> + Send,
    #[cfg(test)] test_migrations: impl MigrationSource<Sqlite> + Send,
) -> Result<Arc<ThreadPool>> {
    let tp = ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("failed to build sqlite threadpool");
    // Connect to the database
    tp.install(|| {
        CONNECTION.with(|c| -> Result<()> {
            log::debug!("connecting to the database at {}", url);
            let mut conn = SqliteConnection::establish(url)?;
            conn.batch_execute("PRAGMA foreign_keys = ON;")?;
            conn.run_pending_migrations(migrations)?;
            #[cfg(test)]
            conn.run_pending_migrations(test_migrations)
                .expect("failed to load test migrations");
            *c.borrow_mut() = Some(conn);
            Ok(())
        })
    })?;
    let arc = Arc::new(tp);
    let cloned = Arc::clone(&arc);
    map.insert(url.clone(), arc);
    Ok(cloned)
}

#[allow(dead_code)]
pub(super) fn cleanup_database(url: &String) {
    let mut map = DB_THREADS.write().unwrap();
    let tp = match map.remove(url) {
        None => return,
        Some(v) => v,
    };
    tp.install(|| {
        CONNECTION.with(|c| {
            *c.borrow_mut() = None;
        })
    });
}

#[macro_export]
macro_rules! def_get_multi {
    ($name:ident, $ret:ty) => {
        fn $name(&self) -> Result<Vec<$ret>>;
    };
}

#[macro_export]
macro_rules! def_delete_by {
    ($name:ident, $sel:ty) => {
        fn $name(&self, sel: $sel) -> Result<()>;
    };
}

#[macro_export]
macro_rules! def_get_one_by {
    ($name:ident, $sel:ty, $ret:ty) => {
        fn $name(&self, sel: $sel) -> Result<$ret>;
    };
}

#[macro_export]
macro_rules! def_get_multi_by {
    ($name:ident, $sel:ty, $ret:ty) => {
        fn $name(&self, sel: $sel) -> Result<Vec<$ret>>;
    };
}

#[macro_export]
macro_rules! def_insert_one {
    ($name:ident, $ty:ty) => {
        fn $name(&self, val: &$ty) -> Result<i32>;
    };
}

#[macro_export]
macro_rules! def_update_one {
    ($name:ident, $ty:ty) => {
        fn $name(&self, val: &$ty) -> Result<()>;
    };
}

#[macro_export]
macro_rules! def_insert_multi {
    ($name:ident, $ty:ty) => {
        fn $name(&self, values: &[$ty]) -> Result<()>;
    };
}

#[macro_export]
macro_rules! def_insert {
    (
        $ins_one:ident,
        $ins_multi:ident,
        $ins_type:ty
    ) => {
        def_insert_one!($ins_one, $ins_type);
        def_insert_multi!($ins_multi, $ins_type);
    };
}

#[macro_export]
macro_rules! def_standard_crud {
    (
        $ins_one:ident,
        $ins_multi:ident,
        $ins_type:ty,
        $get_all:ident,
        $get_by_id:ident,
        $update_one:ident,
        $read_update_type:ty,
        $delete_by_id:ident
    ) => {
        def_insert_one!($ins_one, $ins_type);
        def_insert_multi!($ins_multi, $ins_type);
        def_get_one_by!($get_by_id, i32, $read_update_type);
        def_get_multi!($get_all, $read_update_type);
        def_update_one!($update_one, $read_update_type);
        def_delete_by!($delete_by_id, i32);
    };
}

#[cfg(feature = "trace_db")]
macro_rules! query {
    ($q:expr) => {{
        let __query = $q;
        ::log::trace!(
            "{}",
            diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query)
        );

        __query
    }};
}

#[cfg(not(feature = "trace_db"))]
macro_rules! query {
    ($q:expr) => {
        $q
    };
}

#[macro_export]
macro_rules! impl_delete_by {
     ($vis:vis $name:ident, $sel:ty, $table:ident, $($filter:tt)+) => {
        $vis fn $name(&self, sel: $sel) -> Result<()> {
            self.with_connection(|conn| {
                let __query = ::diesel::delete(
                     super::schema::$table::dsl::$table.filter(
                        super::schema::$table::dsl::$($filter)+(sel)
                    ));
                #[cfg(feature = "trace_db")]
                ::log::trace!("{}", diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query));
                __query.execute(conn)?;
                Ok(())
            })
        }
    }
}

#[macro_export]
macro_rules! impl_get_by {
    ($vis:vis $retrieve:ident, $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $vis fn $name(&self, sel: $sel) -> Result<$ret> {
            self.with_connection(|conn| {
                let __query = super::schema::$ty::dsl::$ty.filter(
                    super::schema::$ty::dsl::$($filter)+(sel)
                );
                #[cfg(feature = "trace_db")]
                log::trace!("{}", diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query));
                Ok(__query.$retrieve(conn)?)
            })
        }
    }
}

#[macro_export]
macro_rules! impl_get {
        ($vis:vis $retrieve:ident, $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $vis fn $name(&self) -> Result<$ret> {
            self.with_connection(|conn| {
                Ok(
                    super::schema::$ty::dsl::$ty.filter(
                        super::schema::$ty::dsl::$($filter)+
                    ).$retrieve(conn)?
                )
            })
        }
    }
}

#[macro_export]
macro_rules! impl_get_one {
    ($vis:vis $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get!($vis get_result, $name, $ret, $ty, $($filter)+);
    }
}

#[macro_export]
macro_rules! impl_get_multi {
    ($vis:vis $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get!($vis get_results, $name, Vec<$ret>, $ty, $($filter)+);
    }
}

#[macro_export]
macro_rules! impl_get_one_by {
    ($vis:vis $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get_by!($vis get_result, $name, $sel, $ret, $ty, $($filter)+);
    }

}

#[macro_export]
macro_rules! impl_get_multi_by {
    ($vis:vis $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get_by!($vis get_results, $name, $sel, Vec<$ret>, $ty, $($filter)+);
    }
}

#[macro_export]
macro_rules! impl_get_all {
    ($vis:vis $name:ident, $ret:ty, $ty:ident) => {
        $vis fn $name(&self) -> Result<Vec<$ret>> {
            self.with_connection(|conn| Ok(super::schema::$ty::dsl::$ty.load(conn)?))
        }
    };
}

#[macro_export]
macro_rules! impl_update_one {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, value: &$ty) -> Result<()> {
            self.with_connection(|conn| {
                let __query = ::diesel::update(value).set(value);
                #[cfg(feature = "trace_db")]
                ::log::trace!(
                    "{}",
                    diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query)
                );
                __query.execute(conn)?;
                Ok(())
            })
        }
    };
}

#[macro_export]
macro_rules! impl_insert_one {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, values: &$ty) -> Result<i32> {
            self.with_connection(|conn| {
                let __query = ::diesel::insert_into(super::schema::$dsl::dsl::$dsl)
                    .values(values)
                    .returning(super::schema::$dsl::id);
                #[cfg(feature = "trace_db")]
                ::log::trace!(
                    "{}",
                    diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query)
                );
                Ok(__query.get_result(conn)?)
            })
        }
    };
}

#[macro_export]
macro_rules! impl_insert_multi {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, values: &[$ty]) -> Result<()> {
            self.with_connection(|conn| {
                let __query = ::diesel::insert_into(super::schema::$dsl::dsl::$dsl).values(values);
                __query.execute(conn)?;
                Ok(())
            })
        }
    };
}

#[macro_export]
macro_rules! impl_simple_gets {
    ($vis:vis $table:ident, $ty:ty, $get_all:ident, $get_by_id:ident) => {
        impl_get_all!($vis $get_all, $ty, $table);
        impl_get_one_by!($vis $get_by_id, i32, $ty, $table, id.eq);
    };
}

#[macro_export]
macro_rules! impl_standard_crud {
    ($vis:vis $table:ident, $ins_one:ident, $ins_multi:ident, $ins_type:ty, $get_all:ident, $get_by_id:ident, $update_one:ident, $read_update_type:ty, $delete_by_id:ident) => {
        impl_insert_one!($vis $ins_one, $ins_type, $table);
        impl_insert_multi!($vis $ins_multi, $ins_type, $table);
        impl_get_all!($vis $get_all, $read_update_type, $table);
        impl_update_one!($vis $update_one, $read_update_type, $table);
        impl_get_one_by!($vis $get_by_id, i32, $read_update_type, $table, id.eq);
        impl_delete_by!($vis $delete_by_id, i32, $table, id.eq);
    };
}
