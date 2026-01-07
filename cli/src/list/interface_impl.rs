use anyhow::bail;
use clap::Args;
use dtu::db::graph::db::FRAMEWORK_SOURCE;
use dtu::db::graph::models::ClassSearch;
use dtu::db::graph::{get_default_graphdb, ClassSpec, GraphDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::{ensure_prereq, ClassName, DevicePath};
use dtu::DefaultContext;

use crate::parsers::DevicePathValueParser;

#[derive(Args)]
pub struct InterfaceImpl {
    /// The interface name to find implementations for
    #[arg(short, long)]
    class: ClassName,

    /// The APK to look for the implementation in
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

impl InterfaceImpl {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::GraphDatabaseSetup)?;
        let source = self
            .apk
            .as_ref()
            .map(|it| it.as_squashed_str())
            .unwrap_or_else(|| FRAMEWORK_SOURCE);
        let gdb = get_default_graphdb(&ctx)?;
        let is_framework = self.apk.is_none();

        let search = ClassSearch::new(&self.class, Some(source));

        let impls = gdb.find_classes_implementing(&search, None)?;
        if impls.len() > 0 {
            self.show_impls(&impls);
            return Ok(());
        }
        if !is_framework || self.no_fallback {
            bail!("no implementations found");
        }

        let search = ClassSearch::new(&self.class, None);

        let impls = gdb.find_classes_implementing(&search, None)?;
        if impls.len() == 0 {
            bail!("no implementations found");
        }
        self.show_impls(&impls);
        return Ok(());
    }

    fn show_impls(&self, impls: &Vec<ClassSpec>) {
        for imp in impls {
            if self.show_source {
                println!("{}|{}", imp.name, imp.source);
            } else {
                println!("{}", imp.name);
            }
        }
    }
}
