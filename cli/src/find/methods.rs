use std::io::{self, Read};

use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::{
    db::graph::{
        get_default_graphdb,
        models::{FieldAccessOp, FieldSearch, FieldSearchParams},
        GraphDatabase, MethodSearch,
    },
    prereqs::Prereq,
    utils::{ensure_prereq, ClassName},
    Context,
};

use crate::utils::ostr;

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

    /// Find methods that access a given field
    #[command()]
    ByField(ByField),

    /// Find methods by source
    #[command()]
    BySource(BySource),
}

impl Methods {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        match self.command {
            Command::ByField(c) => c.run(ctx),
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

    /// Method source to filter on
    #[arg(short = 'S', long)]
    source: Option<String>,
}

impl ByString {
    fn run(mut self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        if self.string == "-" {
            self.string.truncate(0);
            io::stdin().read_to_string(&mut self.string)?;
        }
        let db = get_default_graphdb(ctx)?;
        let mut methods = db.get_methods_for_string(&self.string)?;
        if let Some(source) = &self.source {
            methods = methods
                .into_iter()
                .filter(|it| it.source == *source)
                .collect::<Vec<_>>();
        }
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

    /// Source to filter on
    #[arg(short = 'S', long)]
    source: Option<String>,
}

impl ByName {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let graphdb = get_default_graphdb(ctx)?;
        let search =
            match MethodSearch::new_from_opts(None, Some(&self.name), None, ostr(&self.source)) {
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

#[derive(Args)]
pub struct ByField {
    /// Class the field belongs to
    #[arg(short, long)]
    class: ClassName,

    /// Field name
    #[arg(short, long)]
    name: String,

    /// Only find write operations
    #[arg(short = 'W', long = "only-write")]
    only_write: bool,

    /// Only find read operations
    #[arg(short = 'R', long = "only-read")]
    only_read: bool,

    #[arg(short = 'S', long)]
    source: Option<String>,
}

impl ByField {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        if self.only_read && self.only_write {
            bail!("can't set both --only-read and --only-write");
        }
        let db = get_default_graphdb(ctx)?;
        let search = FieldSearch::new(
            FieldSearchParams::new(&self.class, Some(&self.name), None)
                .map_err(|_| anyhow::Error::msg("invalid args"))?,
            ostr(&self.source),
        );
        let fields = db.get_field_ids(&search)?;

        let mut methods = Vec::new();
        let action = if self.only_write {
            Some(FieldAccessOp::Write)
        } else if self.only_read {
            Some(FieldAccessOp::Read)
        } else {
            None
        };

        for f in fields {
            let res = db.get_methods_referencing_field(f, action)?;
            if res.len() > 0 {
                methods.extend(res);
            }
        }

        serde_json::to_writer(io::stdout(), &methods)?;
        Ok(())
    }
}
