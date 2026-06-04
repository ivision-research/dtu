use std::io;

use anyhow::bail;
use clap::Args;
use dtu::db::graph::models::ClassSearch;
use dtu::db::graph::{get_default_graphdb, ClassSpec, GraphDatabase};
use dtu::db::graph::{DefaultGraphDatabase, FRAMEWORK_SOURCE};
use dtu::prereqs::Prereq;
use dtu::utils::{ensure_prereq, ClassName};
use dtu::DefaultContext;

use crate::parsers::GraphSourceValueParser;

struct Common {
    class: ClassName,
    source: Option<String>,
    json: bool,
    no_fallback: bool,
    show_source: bool,
}

impl Common {
    pub fn run<F>(self, get: F) -> anyhow::Result<()>
    where
        F: Fn(&ClassName, Option<&str>, &DefaultGraphDatabase) -> anyhow::Result<Vec<ClassSpec>>,
    {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::GraphDatabaseSetup)?;
        let source = self
            .source
            .as_ref()
            .map(String::as_str)
            .unwrap_or_else(|| FRAMEWORK_SOURCE);

        let gdb = get_default_graphdb(&ctx)?;
        let is_framework = self.source.is_none();

        let classes = get(&self.class, Some(source), &gdb)?;
        if classes.len() > 0 {
            self.show_classes(classes)?;
            return Ok(());
        }

        if !is_framework || self.no_fallback {
            bail!("no child classes found");
        }

        let classes = get(&self.class, None, &gdb)?;
        if classes.len() == 0 {
            bail!("no classes found");
        }
        self.show_classes(classes)?;
        Ok(())
    }

    fn show_classes(&self, classes: Vec<ClassSpec>) -> anyhow::Result<()> {
        if self.json {
            serde_json::to_writer(io::stdout(), &classes)?;
            return Ok(());
        }

        for imp in classes {
            if self.show_source {
                println!("{}|{}", imp.name, imp.source);
            } else {
                println!("{}", imp.name);
            }
        }
        Ok(())
    }
}

#[derive(Args)]
pub struct Children {
    /// The parent class name
    #[arg(short, long)]
    class: ClassName,

    /// The graph source to look for the child classes in
    ///
    /// If this isn't set, it is assumed that the framework is to be searched.
    /// If nothing is found in the framework, and `--no-fallback` isn't set,
    /// all APKs will be searched
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,

    /// JSON output
    #[arg(short, long)]
    json: bool,

    /// Don't fall back to searching for APKs if `--source` isn't set
    #[arg(long)]
    no_fallback: bool,

    /// Also print the source
    #[arg(long)]
    show_source: bool,
}

impl Children {
    pub fn run(self) -> anyhow::Result<()> {
        let common = Common::from(self);
        common.run(|class, source, gdb| {
            let search = ClassSearch::new(class, source);
            Ok(gdb.find_child_classes_of(&search, None)?)
        })
    }
}

impl From<Children> for Common {
    fn from(value: Children) -> Self {
        Self {
            class: value.class,
            source: value.source,
            json: value.json,
            no_fallback: value.no_fallback,
            show_source: value.show_source,
        }
    }
}

#[derive(Args)]
pub struct InterfaceImpl {
    /// The interface class name
    #[arg(short, long)]
    class: ClassName,

    /// The graph source to look for the implementation classes in
    ///
    /// If this isn't set, it is assumed that the framework is to be searched.
    /// If nothing is found in the framework, and `--no-fallback` isn't set,
    /// all APKs will be searched
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: Option<String>,

    /// JSON output
    #[arg(short, long)]
    json: bool,

    /// Don't fall back to searching for APKs if `--source` isn't set
    #[arg(long)]
    no_fallback: bool,

    /// Also print the source
    #[arg(long)]
    show_source: bool,
}

impl InterfaceImpl {
    pub fn run(self) -> anyhow::Result<()> {
        let common = Common::from(self);
        common.run(|class, source, gdb| {
            let search = ClassSearch::new(class, source);
            Ok(gdb.find_classes_implementing(&search, None)?)
        })
    }
}

impl From<InterfaceImpl> for Common {
    fn from(value: InterfaceImpl) -> Self {
        Self {
            class: value.class,
            source: value.source,
            json: value.json,
            no_fallback: value.no_fallback,
            show_source: value.show_source,
        }
    }
}

#[derive(Args)]
pub struct Parents {
    /// The interface class name
    #[arg(short, long)]
    class: ClassName,

    /// The graph source to look for the initial child class in
    #[arg(short = 'S', long, value_parser = GraphSourceValueParser)]
    source: String,

    /// JSON output
    #[arg(short, long)]
    json: bool,

    /// Also print the source
    #[arg(long)]
    show_source: bool,
}

impl Parents {
    pub fn run(self) -> anyhow::Result<()> {
        let common = Common::from(self);
        common.run(|class, source, gdb| {
            let source =
                source.ok_or_else(|| anyhow::Error::msg("source required for parents search"))?;
            Ok(gdb.find_parent_classes_of(class, source)?)
        })
    }
}

impl From<Parents> for Common {
    fn from(value: Parents) -> Self {
        Self {
            class: value.class,
            source: Some(value.source),
            json: value.json,
            no_fallback: true,
            show_source: value.show_source,
        }
    }
}
