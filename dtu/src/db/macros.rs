//! The declarative macros used to build the database implementations
//!
//! These are not re-exported by [crate::db::common], so every user has to name the ones it
//! wants. Macros invoked from inside another macro here are path qualified so that
//! importing the outer one is enough.

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

#[allow(unused_macros)]
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

#[allow(unused_macros)]
macro_rules! def_insert {
    (
        $ins_one:ident,
        $ins_multi:ident,
        $ins_type:ty
    ) => {
        $crate::db::macros::def_insert_one!($ins_one, $ins_type);
        $crate::db::macros::def_insert_multi!($ins_multi, $ins_type);
    };
}

#[allow(unused)]
pub(super) use def_insert;

#[allow(unused_macros)]
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
        $crate::db::macros::def_insert_one!($ins_one, $ins_type);
        $crate::db::macros::def_insert_multi!($ins_multi, $ins_type);
        $crate::db::macros::def_get_one_by!($get_by_id, i32, $read_update_type);
        $crate::db::macros::def_get_multi!($get_all, $read_update_type);
        $crate::db::macros::def_update_one!($update_one, $read_update_type);
        $crate::db::macros::def_delete_by!($delete_by_id, i32);
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
            let __query = $crate::db::macros::query!(::diesel::delete(
                super::schema::$table::dsl::$table.filter(
                    super::schema::$table::dsl::$($filter)+(sel)
            )));
            self.write(|conn| {
                __query.execute(conn)?;
                Ok(())
            })
        }
    }
}

pub(super) use impl_delete_by;

macro_rules! impl_get_by {
    ($vis:vis $retrieve:ident, $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $vis fn $name(&self, sel: $sel) -> Result<$ret> {
            let __query = $crate::db::macros::query!(super::schema::$ty::dsl::$ty.filter(
                super::schema::$ty::dsl::$($filter)+(sel)
            ));
            self.query(|conn| Ok(__query.$retrieve(conn)?))
        }
    }
}

pub(super) use impl_get_by;

macro_rules! impl_get {
        ($vis:vis $retrieve:ident, $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $vis fn $name(&self) -> Result<$ret> {
            let __query = $crate::db::macros::query!(super::schema::$ty::dsl::$ty.filter(
                super::schema::$ty::dsl::$($filter)+
            ));
            self.query(|conn| Ok(__query.$retrieve(conn)?))
        }
    }
}

pub(super) use impl_get;

#[allow(unused_macros)]
macro_rules! impl_get_one {
    ($vis:vis $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $crate::db::macros::impl_get!($vis get_result, $name, $ret, $ty, $($filter)+);
    }
}

#[allow(unused)]
pub(super) use impl_get_one;

macro_rules! impl_get_multi {
    ($vis:vis $name:ident, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $crate::db::macros::impl_get!($vis get_results, $name, Vec<$ret>, $ty, $($filter)+);
    }
}

pub(super) use impl_get_multi;

macro_rules! impl_get_one_by {
    ($vis:vis $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $crate::db::macros::impl_get_by!($vis get_result, $name, $sel, $ret, $ty, $($filter)+);
    }

}

pub(super) use impl_get_one_by;

macro_rules! impl_get_multi_by {
    ($vis:vis $name:ident, $sel:ty, $ret:ty, $ty:ident, $($filter:tt)+) => {
        $crate::db::macros::impl_get_by!($vis get_results, $name, $sel, Vec<$ret>, $ty, $($filter)+);
    }
}

pub(super) use impl_get_multi_by;

macro_rules! impl_get_all {
    ($vis:vis $name:ident, $ret:ty, $ty:ident) => {
        $vis fn $name(&self) -> Result<Vec<$ret>> {
            let __query = $crate::db::macros::query!(super::schema::$ty::dsl::$ty);
            self.query(|conn| Ok(__query.load(conn)?))
        }
    };
}

pub(super) use impl_get_all;

macro_rules! impl_update_one {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, value: &$ty) -> Result<()> {
            let __query = $crate::db::macros::query!(::diesel::update(value).set(value));
            self.write(|conn| {
                __query.execute(conn)?;
                Ok(())
            })
        }
    };
}

pub(super) use impl_update_one;

macro_rules! impl_insert_one {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, values: &$ty) -> Result<i32> {
            let __query = $crate::db::macros::query!(::diesel::insert_into(super::schema::$dsl::dsl::$dsl)
                .values(values)
                .returning(super::schema::$dsl::id));
            self.write(|conn| Ok(__query.get_result(conn)?))
        }
    };
}

pub(super) use impl_insert_one;

macro_rules! impl_insert_multi {
    ($vis:vis $name:ident, $ty:ty, $dsl:ident) => {
        $vis fn $name(&self, values: &[$ty]) -> Result<()> {
            let __query = $crate::db::macros::query!(::diesel::insert_into(super::schema::$dsl::dsl::$dsl).values(values));
            self.write(|conn| {
                __query.execute(conn)?;
                Ok(())
            })
        }
    };
}

pub(super) use impl_insert_multi;

macro_rules! impl_simple_gets {
    ($vis:vis $table:ident, $ty:ty, $get_all:ident, $get_by_id:ident) => {
        $crate::db::macros::impl_get_all!($vis $get_all, $ty, $table);
        $crate::db::macros::impl_get_one_by!($vis $get_by_id, i32, $ty, $table, id.eq);
    };
}

pub(super) use impl_simple_gets;

#[allow(unused_macros)]
macro_rules! impl_standard_crud {
    ($vis:vis $table:ident, $ins_one:ident, $ins_multi:ident, $ins_type:ty, $get_all:ident, $get_by_id:ident, $update_one:ident, $read_update_type:ty, $delete_by_id:ident) => {
        $crate::db::macros::impl_insert_one!($vis $ins_one, $ins_type, $table);
        $crate::db::macros::impl_insert_multi!($vis $ins_multi, $ins_type, $table);
        $crate::db::macros::impl_get_all!($vis $get_all, $read_update_type, $table);
        $crate::db::macros::impl_update_one!($vis $update_one, $read_update_type, $table);
        $crate::db::macros::impl_get_one_by!($vis $get_by_id, i32, $read_update_type, $table, id.eq);
        $crate::db::macros::impl_delete_by!($vis $delete_by_id, i32, $table, id.eq);
    };
}

#[allow(unused)]
pub(super) use impl_standard_crud;
