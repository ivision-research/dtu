use std::io::{self, Read};

use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::{
    db::graph::{get_default_graphdb, GraphDatabase, MethodSearch},
    prereqs::Prereq,
    utils::{ensure_prereq, ClassName},
    Context,
};

#[derive(Args)]
pub struct Methods {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Search by string inclusion
    #[command()]
    ByString(ByString),

    /// Find methods by name
    #[command()]
    ByName(ByName),

    /// Find methods by class name
    #[command()]
    ByClass(ByClass),

    /// Find methods by source
    #[command()]
    BySource(BySource),
}

impl Methods {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        match self.command {
            Command::ByString(c) => c.run(ctx),
            Command::ByClass(c) => c.run(ctx),
            Command::ByName(c) => c.run(ctx),
            Command::BySource(c) => c.run(ctx),
        }
    }
}

#[derive(Args)]
struct ByString {
    /// String to search for, can be `-` for stdin
    #[arg()]
    string: String,
}

impl ByString {
    fn run(mut self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        if self.string == "-" {
            self.string.truncate(0);
            io::stdin().read_to_string(&mut self.string)?;
        }
        let db = get_default_graphdb(ctx)?;
        let methods = db.get_methods_for_string(&self.string)?;
        serde_json::to_writer(io::stdout(), &methods)?;
        Ok(())
    }
}

#[derive(Args)]
struct BySource {
    #[arg(short = 'S', long)]
    source: String,
}

impl BySource {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let methods = db.get_methods_for(&self.source)?;
        serde_json::to_writer(io::stdout(), &methods)?;
        Ok(())
    }
}

#[derive(Args)]
struct ByName {
    #[arg(short, long)]
    name: String,
}

impl ByName {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let graphdb = get_default_graphdb(ctx)?;
        let search = match MethodSearch::new_from_opts(None, Some(&self.name), None, None) {
            Ok(v) => v,
            Err(e) => bail!("{e}"),
        };

        let methods = graphdb.get_methods(&search)?;
        serde_json::to_writer(io::stdout(), &methods)?;
        Ok(())
    }
}

#[derive(Args)]
struct ByClass {
    #[arg(short, long)]
    class: ClassName,
}

impl ByClass {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let graphdb = get_default_graphdb(ctx)?;
        let search = match MethodSearch::new_from_opts(Some(&self.class), None, None, None) {
            Ok(v) => v,
            Err(e) => bail!("{e}"),
        };

        let methods = graphdb.get_methods(&search)?;
        serde_json::to_writer(io::stdout(), &methods)?;
        Ok(())
    }
}
