use anyhow::bail;
use clap::Args;
use dtu::db::graph::db::FRAMEWORK_SOURCE;
use dtu::db::graph::{get_default_graphdb, ClassMeta, GraphDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::{ensure_prereq, ClassName, DevicePath};
use dtu::DefaultContext;

use crate::parsers::DevicePathValueParser;

#[derive(Args)]
pub struct Children {
    /// The parent class name
    #[arg(short, long)]
    class: ClassName,

    /// The APK to look for the child classes in
    ///
    /// If this isn't set, it is assumed that the framework is to be searched.
    /// If nothing is found in the framework, and `--no-fallback` isn't set,
    /// all APKs will be searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Don't fall back to searching for APKs if `--apk` isn't set
    #[arg(long)]
    no_fallback: bool,

    /// Also print the source
    #[arg(long)]
    show_source: bool,
}

impl Children {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::GraphDatabasePartialSetup)?;
        let source = self
            .apk
            .as_ref()
            .map(|it| it.as_squashed_str())
            .unwrap_or_else(|| FRAMEWORK_SOURCE);

        let gdb = get_default_graphdb(&ctx)?;
        let is_framework = self.apk.is_none();
        let children = gdb.find_child_classes_of(&self.class, Some(source))?;
        if children.len() > 0 {
            self.show_children(&children);
            return Ok(());
        }

        if !is_framework || self.no_fallback {
            bail!("no child classes found");
        }

        let children = gdb.find_classes_implementing(&self.class, None)?;
        if children.len() == 0 {
            bail!("no child classes found");
        }
        self.show_children(&children);
        Ok(())
    }

    fn show_children(&self, children: &Vec<ClassMeta>) {
        for imp in children {
            if self.show_source {
                println!("{}|{}", imp.name, imp.source);
            } else {
                println!("{}", imp.name);
            }
        }
    }
}
