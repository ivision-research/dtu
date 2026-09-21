//! We Java now!

use std::{
    marker::PhantomData,
    sync::atomic::{AtomicI32, Ordering},
};

use crate::db::DatabaseId;

pub struct IdFactory<T> {
    current: AtomicI32,

    _ph: PhantomData<T>,
}

impl<T: DatabaseId> IdFactory<T> {
    pub fn new(highest: Option<i32>) -> Self {
        Self {
            current: AtomicI32::new(highest.unwrap_or(0) + 1),
            _ph: PhantomData,
        }
    }

    pub fn next(&self) -> T {
        let next = self.current.fetch_add(1, Ordering::Relaxed);
        T::from_id(next)
    }
}

/// Build an [IdFactory] seeded from `MAX(column)` over `table`
macro_rules! new_id_factory {
    ($db:expr, $table:path, $column:path) => {{
        let highest = $db.query(|c| {
            $crate::db::query!($table
                .select(::diesel::dsl::max($column)))
                .get_result::<Option<i32>>(c)
        });
        highest.map($crate::analysis::taint::id_factory::IdFactory::new)
    }};
}

pub(super) use new_id_factory;
