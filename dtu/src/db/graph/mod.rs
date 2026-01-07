pub mod db;
pub mod models;

use crate::Context;

#[cfg(feature = "setup")]
pub mod setup;
#[cfg(feature = "setup")]
pub use setup::*;

pub use db::{Error, GraphDatabase, Result};

pub use crate::db::sql::graph::GraphSqliteDatabase as SqliteGraphDatabase;

pub use models::ClassSpec;

pub type DefaultGraphDatabase = SqliteGraphDatabase;

/// Get the default GraphDB implementation
pub fn get_default_graphdb(ctx: &dyn Context) -> Result<SqliteGraphDatabase> {
    Ok(SqliteGraphDatabase::new(&ctx)?)
}
