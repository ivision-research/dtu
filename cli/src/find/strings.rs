use std::io;

use anyhow::bail;
use clap::{self, Args, Subcommand};
use dtu::{
    db::graph::{get_default_graphdb, GraphDatabase, MethodSearch, MethodSpec, FRAMEWORK_SOURCE},
    prereqs::Prereq,
    utils::{ensure_prereq, ClassName, DevicePath},
    Context,
};

use crate::{parsers::DevicePathValueParser, utils::ostr};

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
    ByMethod(ByMethod),

    #[command()]
    MethodsByStr(MethodsByString),
}

impl Strings {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        match self.command {
            Command::BySource(c) => c.run(ctx),
            Command::ByMethod(c) => c.run(ctx),
            Command::MethodsByStr(c) => c.run(ctx),
        }
    }
}

#[derive(Args)]
struct MethodsByString {
    #[arg()]
    string: String,
}

impl MethodsByString {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let result = db.get_methods_for_string(&self.string)?;
        serde_json::to_writer(io::stdout(), &result)?;
        Ok(())
    }
}

#[derive(Args)]
struct BySource {
    /// The APK to search, otherwise the framework source is used
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,
}

impl BySource {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
        let db = get_default_graphdb(ctx)?;
        let src = self.get_source();
        let strings = db.get_strings_for_source(src)?;
        for s in strings {
            println!("{s}");
        }
        Ok(())
    }

    fn get_source(&self) -> &str {
        if let Some(ref apk) = self.apk {
            apk.as_squashed_str()
        } else {
            FRAMEWORK_SOURCE
        }
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
    #[arg(short = 'S', long)]
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
