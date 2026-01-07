#[cfg(feature = "cozo")]
pub mod cozodb;
#[cfg(all(feature = "cozo", feature = "setup"))]
pub(crate) mod cozosetup;
#[cfg(feature = "cozo")]
pub(crate) mod cozoutils;

pub mod db;
pub mod models;

use crate::Context;

#[cfg(feature = "setup")]
pub mod setup;
#[cfg(feature = "setup")]
pub use setup::*;

#[cfg(feature = "cozo")]
pub use cozodb::CozoGraphDatabase;

pub use db::{Error, GraphDatabase, Result};

pub use models::ClassMeta;

pub enum GraphDatabaseOptions {
    #[cfg(feature = "cozo")]
    Cozo,
    Empty,
}

impl GraphDatabaseOptions {
    /// Return the options to build a new default [GraphDatabase] implementation
    #[allow(unused_variables)]
    #[cfg(feature = "cozo")]
    pub fn new_default(ctx: &dyn Context) -> Self {
        return Self::new_cozo();
    }

    #[cfg(feature = "cozo")]
    pub fn new_cozo() -> Self {
        Self::Cozo
    }

    /// Consume the options and create a [GraphDatabase] impl
    pub fn get_db(self, ctx: &dyn Context) -> Result<Box<dyn GraphDatabase>> {
        Ok(match self {
            #[cfg(feature = "cozo")]
            GraphDatabaseOptions::Cozo => Box::new(CozoGraphDatabase::new(ctx)?),
            _ => {
                return Err(Error::Generic(
                    "no default graph database implementation supported".into(),
                ))
            }
        })
    }
}

#[cfg(feature = "cozo")]
pub type DefaultGraphDatabase = CozoGraphDatabase;

/// Get the default GraphDB implementation
#[cfg(feature = "cozo")]
pub fn get_default_graphdb(ctx: &dyn Context) -> Result<CozoGraphDatabase> {
    CozoGraphDatabase::new(&ctx)
}
