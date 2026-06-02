use std::io::{self};

use clap::{self, Args, Subcommand};
use dtu::{
    Context, db::graph::{
        GraphDatabase, MethodSearch, get_default_graphdb, models::{FieldAccessOp, FieldSearch, FieldSearchParams}
    }, prereqs::Prereq, utils::{ClassName, ensure_prereq}
};

use crate::utils::ostr;

#[derive(Args)]
pub struct Fields {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find fields according to class, name, and source
    #[command()]
    BySpec(BySpec),

    /// Find all fields referenced by the given method
    #[command()]
    ByMethod(ByMethod),
}

impl Fields {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        match self.command {
            Command::BySpec(c) => c.run(ctx),
            Command::ByMethod(c) => c.run(ctx),
        }
    }
}

#[derive(Args)]
pub struct ByMethod {
    /// Class containing the method
    #[arg(short, long)]
    class: ClassName,

    /// Method name
    #[arg(short, long)]
    name: Option<String>,

    /// Method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// Only find write operations
    #[arg(short = 'W', long = "only-write")]
    only_write: bool,

    /// Only find read operations
    #[arg(short = 'R', long = "only-read")]
    only_read: bool,

    /// Source containing the class
    #[arg(short = 'S', long)]
    source: Option<String>,
}

impl ByMethod {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let search = MethodSearch::new_from_opts(
            Some(&self.class),
            ostr(&self.name),
            ostr(&self.signature),
            ostr(&self.source),
        )
        .map_err(|e| anyhow::Error::msg(e))?;

        let methods = db.get_method_ids(&search)?;
        let mut fields = Vec::new();

        let action = if self.only_write {
            Some(FieldAccessOp::Write)
        } else if self.only_read {
            Some(FieldAccessOp::Read)
        } else {
            None
        };

        for m in methods {
            let refs = db.get_method_field_refs(m)?;
            if refs.len() > 0 {
                fields.extend(refs.into_iter().filter_map(|it| {
                    if let Some(a) = action {
                        if it.op != a {
                            return None;
                        }
                    }
                    Some(it.field)
                }));
            }
        }

        serde_json::to_writer(io::stdout(), &fields)?;
        Ok(())
    }
}

#[derive(Args)]
pub struct BySpec {
    /// Class containing the field
    #[arg(short, long)]
    class: ClassName,

    /// Field name
    #[arg(short, long)]
    name: Option<String>,

    /// Source containing the class
    #[arg(short = 'S', long)]
    source: Option<String>,
}

impl BySpec {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let search = FieldSearch::new(
            FieldSearchParams::new(&self.class, ostr(&self.name), None)
                .map_err(|_| anyhow::Error::msg("invalid args"))?,
            ostr(&self.source),
        );
        let fields = db.get_fields(&search)?;
        serde_json::to_writer(io::stdout(), &fields)?;
        Ok(())
    }
}
