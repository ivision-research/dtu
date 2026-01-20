use clap::{self, Args, Subcommand};
use dtu::db::meta::get_default_metadb;
use dtu::db::MetaDatabase;
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

use crate::parsers::DevicePathValueParser;
use dtu::db::graph::{get_default_graphdb, GraphDatabase};
use dtu::utils::DevicePath;

mod import_apk_methods;
mod monitor;
mod setup;

#[derive(Args)]
pub struct Graph {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set up the graph database, this may take some time
    #[command(alias = "full-setup")]
    Setup(setup::Setup),

    /// Wipe the graph database
    #[command()]
    Wipe,

    /// Add an APK's methods to the existing database
    #[command()]
    AddApkMethods(import_apk_methods::ImportApkMethods),

    /// Remove a graph database source
    #[command()]
    RemoveSource(RemoveSource),
}

impl Graph {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::Setup(c) => c.run(),
            Command::AddApkMethods(c) => c.run(),
            Command::RemoveSource(c) => c.run(),
            Command::Wipe => self.wipe(),
        }
    }

    fn wipe(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx)?;
        let db = get_default_graphdb(&ctx)?;
        db.wipe(&ctx)?;
        meta.update_prereq(Prereq::GraphDatabaseSetup, false)?;
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
