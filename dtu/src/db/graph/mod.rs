#[cfg(feature = "cozo")]
pub mod cozodb;
#[cfg(all(feature = "cozo", feature = "setup"))]
pub(crate) mod cozosetup;
#[cfg(feature = "cozo")]
pub(crate) mod cozoutils;

pub mod db;
pub mod models;
#[cfg(feature = "neo4j")]
pub mod neo4j;

use crate::Context;

#[cfg(feature = "setup")]
pub mod setup;
#[cfg(feature = "setup")]
pub use setup::*;

#[cfg(feature = "cozo")]
pub use cozodb::CozoGraphDatabase;

pub use db::{Error, GraphDatabase, Result};

pub use models::ClassMeta;
#[cfg(feature = "neo4j")]
pub use neo4j::Neo4jDatabase;

#[cfg(feature = "neo4j")]
pub struct Neo4jOptions {
    uri: String,
    user: String,
    pass: String,
}

pub enum GraphDatabaseOptions {
    #[cfg(feature = "neo4j")]
    Neo4j(Neo4jOptions),
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

    #[cfg(all(not(feature = "cozo"), feature = "neo4j"))]
    pub fn new_default(ctx: &dyn Context) -> Self {
        return Self::new_neo4j(ctx);
    }

    #[cfg(all(not(feature = "cozo"), not(feature = "neo4j")))]
    pub fn new_default(ctx: &dyn Context) -> Self {
        return Self::Empty;
    }

    #[cfg(feature = "neo4j")]
    pub fn new_neo4j(ctx: &dyn Context) -> Self {
        let uri = ctx
            .maybe_get_env("DTU_NEO4J_URI")
            .unwrap_or_else(|| String::from("127.0.0.1:7687"));
        let user = ctx
            .maybe_get_env("DTU_NEO4J_USER")
            .unwrap_or_else(|| String::from(""));
        let pass = ctx
            .maybe_get_env("DTU_NEO4J_PASSWORD")
            .unwrap_or_else(|| String::from(""));
        Self::Neo4j(Neo4jOptions { uri, user, pass })
    }

    #[cfg(feature = "cozo")]
    pub fn new_cozo() -> Self {
        Self::Cozo
    }

    /// Consume the options and create a [GraphDatabase] impl
    pub fn get_db(self, ctx: &dyn Context) -> Result<Box<dyn GraphDatabase>> {
        Ok(match self {
            #[cfg(feature = "neo4j")]
            GraphDatabaseOptions::Neo4j(n4j) => {
                Box::new(Neo4jDatabase::connect(&n4j.uri, &n4j.user, &n4j.pass)?)
            }
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

#[cfg(all(not(feature = "cozo"), feature = "neo4j"))]
pub type DefaultGraphDatabase = Neo4jDatabase;

#[cfg(all(not(feature = "cozo"), feature = "neo4j"))]
pub fn get_default_graphdb(ctx: &dyn Context) -> Result<Neo4jDatabase> {
    let opts = GraphDatabaseOptions::new_neo4j(ctx);
    Ok(Neo4jDatabase::connect(&opts.uri, &opts.user, &opts.pass)?)
}
