use clap::{self, Args, Subcommand};
use dtu::db::sql::meta::get_default_metadb;
use dtu::db::sql::MetaDatabase;
use dtu::prereqs::Prereq;
use dtu::DefaultContext;
#[cfg(feature = "neo4j")]
mod neo4j;
#[cfg(feature = "neo4j")]
use neo4j::*;

use crate::parsers::DevicePathValueParser;
use dtu::db::graph::{get_default_graphdb, GraphDatabase};
use dtu::utils::DevicePath;

mod canned;
mod import_apk_methods;
mod monitor;
mod repl;
mod setup;

#[derive(Args)]
pub struct Graph {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[cfg(feature = "neo4j")]
    /// Start the graph docker container
    #[command()]
    StartNeo4j(StartNeo4j),

    #[cfg(feature = "neo4j")]
    /// Stop the graph docker container
    #[command()]
    StopNeo4j(StopNeo4j),

    /// Partially set up the graph database, this only sets up what is needed
    /// to create the SQLite database
    #[command()]
    Setup(setup::Setup),

    /// Fully set up the graph database, this takes a while but adds all methods
    /// and calls to the database as well
    #[command()]
    FullSetup(setup::FullSetup),

    /// Optimize the database, this may be a no-op for some database backends
    #[command()]
    Optimize,

    /// Wipe the graph database
    #[command()]
    Wipe,

    /// Add an APK's methods to the existing database
    #[command()]
    AddApkMethods(import_apk_methods::ImportApkMethods),

    /// Remove a graph database source
    #[command()]
    RemoveSource(RemoveSource),

    /// Run some predefined queries against the Graph database
    #[command()]
    Canned(canned::Canned),

    /// Run a REPL for the default Graph database
    #[command()]
    Repl(repl::Repl),

    /// Execute a single script against the default Graph database
    #[command()]
    Eval(repl::Eval),
}

impl Graph {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            #[cfg(feature = "neo4j")]
            Command::StartNeo4j(c) => c.run(),
            #[cfg(feature = "neo4j")]
            Command::StopNeo4j(c) => c.run(),
            Command::Setup(c) => c.run(),
            Command::FullSetup(c) => c.run(),
            Command::AddApkMethods(c) => c.run(),
            Command::RemoveSource(c) => c.run(),
            Command::Canned(c) => c.run(),
            Command::Optimize => self.optimize(),
            Command::Wipe => self.wipe(),
            Command::Repl(c) => c.run(),
            Command::Eval(c) => c.run(),
        }
    }

    fn optimize(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = get_default_graphdb(&ctx)?;
        db.optimize()?;
        Ok(())
    }

    fn wipe(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx)?;
        let db = get_default_graphdb(&ctx)?;
        db.wipe(&ctx)?;
        meta.update_prereq(Prereq::GraphDatabasePartialSetup, false)?;
        meta.update_prereq(Prereq::GraphDatabaseFullSetup, false)?;
        Ok(())
    }
}

#[derive(Args)]
struct RemoveSource {
    /// The APK source to remove
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: DevicePath,
}

impl RemoveSource {
    fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = get_default_graphdb(&ctx)?;
        db.remove_source(self.apk.as_squashed_str())?;
        Ok(())
    }
}
