pub mod db;
pub mod models;
pub mod schema;

use super::common;

pub type DefaultMetaDatabase = db::MetaSqliteDatabase;

/// Get the default [MetaDatabase] implementation
pub fn get_default_metadb(ctx: &dyn crate::Context) -> common::Result<DefaultMetaDatabase> {
    db::MetaSqliteDatabase::new(ctx)
}
