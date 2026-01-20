pub mod db;
pub mod models;

pub mod schema;
use super::common;

mod traitdef;
pub use traitdef::*;

pub use models::{
    ClassSearch, ClassSpec, MethodCallPath, MethodSearch, MethodSearchParams, MethodSpec,
};

#[cfg(feature = "setup")]
mod setup;

#[cfg(feature = "setup")]
mod setup_task;
#[cfg(feature = "setup")]
pub use setup_task::*;

pub use db::GraphSqliteDatabase;

pub type DefaultGraphDatabase = GraphSqliteDatabase;

/// Get the default GraphDB implementation
pub fn get_default_graphdb(ctx: &dyn crate::Context) -> super::common::Result<GraphSqliteDatabase> {
    Ok(GraphSqliteDatabase::new(&ctx)?)
}
