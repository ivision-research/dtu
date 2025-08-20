pub mod db;
pub mod models;
pub mod schema;

use super::common;

/// Get the default [MetaDatabase] implementation
pub fn get_default_metadb(ctx: &dyn crate::Context) -> common::Result<db::MetaSqliteDatabase> {
    db::MetaSqliteDatabase::new(ctx)
}
