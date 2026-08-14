#![allow(unused_macros)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
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

/// Create a database ID type with the given name and doc comment
///
/// The returned type is just a wrapper around i32s and used for type safety
macro_rules! database_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Copy,
            Clone,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            Debug,
            serde::Serialize,
            serde::Deserialize,
            diesel::expression::AsExpression,
            diesel::FromSqlRow,
        )]
        #[diesel(sql_type = diesel::sql_types::Integer)]
        #[serde(transparent)]
        pub struct $name(i32);

        impl $name {
            pub const fn new(id: i32) -> Self {
                Self(id)
            }
            pub const fn raw(self) -> i32 {
                self.0
            }
        }

        impl crate::db::common::DatabaseId for $name {
            fn from_id(id: i32) -> Self {
                Self::new(id)
            }
            fn id(self) -> i32 {
                self.raw()
            }
        }

        impl From<i32> for $name {
            fn from(id: i32) -> Self {
                Self(id)
            }
        }

        impl From<$name> for i32 {
            fn from(id: $name) -> i32 {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl diesel::serialize::ToSql<diesel::sql_types::Integer, diesel::sqlite::Sqlite>
            for $name
        {
            fn to_sql<'b>(
                &'b self,
                out: &mut diesel::serialize::Output<'b, '_, diesel::sqlite::Sqlite>,
            ) -> diesel::serialize::Result {
                out.set_value(self.0);
                Ok(diesel::serialize::IsNull::No)
            }
        }

        impl diesel::deserialize::FromSql<diesel::sql_types::Integer, diesel::sqlite::Sqlite>
            for $name
        {
            fn from_sql(
                value: diesel::sqlite::SqliteValue<'_, '_, '_>,
            ) -> diesel::deserialize::Result<Self> {
                <i32 as diesel::deserialize::FromSql<
                    diesel::sql_types::Integer,
                    diesel::sqlite::Sqlite,
                >>::from_sql(value)
                .map(Self)
            }
        }
    };
}

pub(super) use database_id;

macro_rules! def_get_multi {
    ($name:ident, $ret:ty) => {
        fn $name(&self) -> Result<Vec<$ret>>;
    };
}

pub(super) use def_get_multi;

macro_rules! def_delete_by {
    ($name:ident, $sel:ty) => {
        fn $name(&self, sel: $sel) -> Result<()>;
    };
}

pub(super) use def_delete_by;

macro_rules! def_get_one_by {
    ($name:ident, $sel:ty, $ret:ty) => {
        fn $name(&self, sel: $sel) -> Result<$ret>;
    };
}

pub(super) use def_get_one_by;

macro_rules! def_get_multi_by {
    ($name:ident, $sel:ty, $ret:ty) => {
        fn $name(&self, sel: $sel) -> Result<Vec<$ret>>;
    };
}

#[allow(unused)]
pub(super) use def_get_multi_by;

macro_rules! def_insert_one {
    ($name:ident, $ty:ty) => {
        fn $name(&self, val: &$ty) -> Result<i32>;
    };
}

pub(super) use def_insert_one;

macro_rules! def_update_one {
    ($name:ident, $ty:ty) => {
        fn $name(&self, val: &$ty) -> Result<()>;
    };
}

pub(super) use def_update_one;

macro_rules! def_insert_multi {
    ($name:ident, $ty:ty) => {
        fn $name(&self, values: &[$ty]) -> Result<()>;
    };
}

pub(super) use def_insert_multi;

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

#[allow(unused)]
pub(super) use def_insert;

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

#[allow(unused)]
pub(super) use def_standard_crud;

#[cfg(feature = "trace_db")]
macro_rules! query {
    ($q:expr) => {{
        let __dbg_query = $q;
        ::log::trace!(
            "{}",
            diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__dbg_query)
        );

        __dbg_query
    }};
}

#[cfg(not(feature = "trace_db"))]
macro_rules! query {
    ($q:expr) => {
        $q
    };
}

pub(crate) use query;

macro_rules! impl_delete_by {
     ($vis:vis $name:ident, $sel:ty, $table:ident, $($filter:tt)+) => {
        $vis fn $name(&self, sel: $sel) -> Result<()> {
            let __query = query!(::diesel::delete(
                super::schema::$table::dsl::$table.filter(
                    super::schema::$table::dsl::$($filter)+(sel)
            )));
            self.with_connection(|conn| {
                __query.execute(conn)
            })?;
            Ok(())
        }
    }
}

pub(super) use impl_delete_by;

macro_rules! impl_get_by {
    ($vis:vis $retrieve:ident, $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $vis fn $name(&self, sel: $sel) -> Result<$ret> {
            let __query = query!(super::schema::$ty::dsl::$ty.filter(
                super::schema::$ty::dsl::$($filter)+(sel)
            ));
            Ok(self.with_connection(|conn| {
                __query.$retrieve(conn)
            })?)
        }
    }
}

pub(super) use impl_get_by;

macro_rules! impl_get {
        ($vis:vis $retrieve:ident, $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $vis fn $name(&self) -> Result<$ret> {
            let __query = query!(super::schema::$ty::dsl::$ty.filter(
                super::schema::$ty::dsl::$($filter)+
            ));
            Ok(self.with_connection(|conn| { __query.$retrieve(conn)})?)
        }
    }
}

pub(super) use impl_get;

macro_rules! impl_get_one {
    ($vis:vis $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get!($vis get_result, $name, $ret, $ty, $($filter)+);
    }
}

#[allow(unused)]
pub(super) use impl_get_one;

macro_rules! impl_get_multi {
    ($vis:vis $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get!($vis get_results, $name, Vec<$ret>, $ty, $($filter)+);
    }
}

pub(super) use impl_get_multi;

macro_rules! impl_get_one_by {
    ($vis:vis $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get_by!($vis get_result, $name, $sel, $ret, $ty, $($filter)+);
    }

}

pub(super) use impl_get_one_by;

macro_rules! impl_get_multi_by {
    ($vis:vis $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        impl_get_by!($vis get_results, $name, $sel, Vec<$ret>, $ty, $($filter)+);
    }
}

pub(super) use impl_get_multi_by;

macro_rules! impl_get_all {
    ($vis:vis $name:ident, $ret:ty, $ty:ident) => {
        $vis fn $name(&self) -> Result<Vec<$ret>> {
            let __query = query!(super::schema::$ty::dsl::$ty);
            Ok(self.with_connection(|conn| __query.load(conn))?)
        }
    };
}

pub(super) use impl_get_all;

macro_rules! impl_update_one {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, value: &$ty) -> Result<()> {
            let __query = query!(::diesel::update(value).set(value));
            self.with_connection(|conn| { __query.execute(conn) })?;
            Ok(())
        }
    };
}

pub(super) use impl_update_one;

macro_rules! impl_insert_one {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, values: &$ty) -> Result<i32> {
            let __query = query!(::diesel::insert_into(super::schema::$dsl::dsl::$dsl)
                .values(values)
                .returning(super::schema::$dsl::id));
            Ok(self.with_connection(|conn| { __query.get_result(conn)})?)
        }
    };
}

pub(super) use impl_insert_one;

macro_rules! impl_insert_multi {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, values: &[$ty]) -> Result<()> {
            let __query = query!(::diesel::insert_into(super::schema::$dsl::dsl::$dsl).values(values));
            self.with_connection(|conn| { __query.execute(conn) })?;
            Ok(())
        }
    };
}

pub(super) use impl_insert_multi;

macro_rules! impl_simple_gets {
    ($vis:vis $table:ident, $ty:ty, $get_all:ident, $get_by_id:ident) => {
        impl_get_all!($vis $get_all, $ty, $table);
        impl_get_one_by!($vis $get_by_id, i32, $ty, $table, id.eq);
    };
}

pub(super) use impl_simple_gets;

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

#[allow(unused)]
pub(super) use impl_standard_crud;
