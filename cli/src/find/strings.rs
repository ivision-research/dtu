use std::io;

use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::{
    db::graph::{
        get_default_graphdb, GraphDatabase, MethodSearch, MethodSpec, StringSearch,
        FRAMEWORK_SOURCE,
    },
    prereqs::Prereq,
    utils::{ensure_prereq, ClassName},
    Context,
};

use crate::{parsers::GraphSourceValueParser, utils::ostr};

#[derive(Args)]
pub struct Strings {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command()]
    BySource(BySource),

    #[command()]
    Like(Like),

    #[command()]
    ByMethod(ByMethod),
}

impl Strings {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        match self.command {
            Command::BySource(c) => c.run(ctx),
            Command::ByMethod(c) => c.run(ctx),
            Command::Like(c) => c.run(ctx),
        }
    }
}

#[derive(Args)]
struct Like {
    /// An optional source to search
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,

    /// String to search for, can be `-` for stdin
    ///
    /// Note that % is interpreted by SQL
    #[arg()]
    string: String,

    #[arg(short, long)]
    json: bool,
}

impl Like {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;

        let search = StringSearch::from(&self.string);
        let strings = db.find_strings(search, ostr(&self.source))?;

        if self.json {
            serde_json::to_writer(io::stdout(), &strings)?;
            return Ok(());
        }

        for s in strings {
            if self.source.is_some() {
                println!("{}", s.string.escape_default());
            } else {
                println!("{} | {}", s.string.escape_default(), s.source);
            }
        }
        Ok(())
    }
}

#[derive(Args)]
struct BySource {
    /// The source to search, defaults to framework if unset
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,
}

impl BySource {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let src = self
            .source
            .as_ref()
            .map(String::as_str)
            .unwrap_or(FRAMEWORK_SOURCE);
        let strings = db.get_strings_for_source(src)?;
        for s in strings {
            println!("{s}");
        }
        Ok(())
    }
}

#[derive(Args)]
struct ByMethod {
    /// The method name
    #[arg(short, long)]
    name: String,

    /// The method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// The method's class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// An optional source
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,
}

impl ByMethod {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let graphdb = get_default_graphdb(ctx)?;
        let search = match MethodSearch::new_from_opts(
            self.class.as_ref(),
            Some(&self.name),
            ostr(&self.signature),
            ostr(&self.source),
        ) {
            Ok(v) => v,
            Err(e) => bail!("{e}"),
        };

        #[derive(serde::Serialize)]
        struct JsonOutput<'a> {
            method: &'a MethodSpec,
            strings: Vec<String>,
        }

        let methods = graphdb.get_methods(&search)?;
        let mut results = Vec::with_capacity(methods.len());

        for m in &methods {
            let strings = graphdb.get_strings_for_method(m.id)?;
            if strings.is_empty() {
                continue;
            }
            results.push(JsonOutput { method: m, strings });
        }

        serde_json::to_writer(io::stdout(), &results)?;
        Ok(())
    }
}
