use std::fs;
use std::path::PathBuf;

use super::monitor::start_print_thread;
use clap::{self, Args};
use crossbeam::channel::{bounded, Sender};
use dtu::db::graph::db::FRAMEWORK_SOURCE;
use dtu::db::graph::{get_default_graphdb, AddDirectoryOptions, GraphDatabase, SetupEvent};
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::tasks::{ChannelEventMonitor, EventMonitor, TaskCancelCheck, TaskCanceller};
use dtu::utils::{opt_deny, path_must_name, Denylist, DevicePath};
use dtu::{Context, DefaultContext};

#[derive(Args)]
pub struct Setup {
    /// Force the database to be reset if it already exists
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    force: bool,

    /// Denylist of APKs
    ///
    /// Some APKs have _ton_ of classes and maybe shouldn't be included in the database. To this
    /// end, you can specify a denylist file for APKs to be excluded. The file should be newline
    /// separated and `#` are treated as comments.
    ///
    /// To deny an APK, put its squashed device path on a single line of this file.
    #[arg(short = 'l', long)]
    apk_denylist_file: Option<PathBuf>,
}

#[derive(Args)]
pub struct FullSetup {
    /// Force the database to be reset if it already exists
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    force: bool,

    /// Denylist of APKs
    ///
    /// Some APKs have _ton_ of classes and maybe shouldn't be included in the database. To this
    /// end, you can specify a denylist file for APKs to be excluded. The file should be newline
    /// separated and `#` are treated as comments.
    ///
    /// To deny an APK, put its squashed device path on a single line of this file.
    #[arg(short = 'l', long)]
    apk_denylist_file: Option<PathBuf>,
}

impl Setup {
    pub fn run(&self) -> anyhow::Result<()> {
        let gs = GenericSetup::new(
            self.force,
            &self.apk_denylist_file,
            true,
            true,
            Prereq::GraphDatabasePartialSetup,
        );
        gs.run()
    }
}

impl FullSetup {
    pub fn run(&self) -> anyhow::Result<()> {
        let gs = GenericSetup::new(
            self.force,
            &self.apk_denylist_file,
            false,
            false,
            Prereq::GraphDatabaseFullSetup,
        );
        gs.run()
    }
}

pub struct GenericSetup<'a> {
    no_calls: bool,
    no_methods: bool,
    force: bool,
    apk_denylist_file: &'a Option<PathBuf>,
    prereq: Prereq,
}

impl<'a> GenericSetup<'a> {
    pub fn new(
        force: bool,
        apk_denylist_file: &'a Option<PathBuf>,
        no_calls: bool,
        no_methods: bool,
        prereq: Prereq,
    ) -> Self {
        Self {
            force,
            apk_denylist_file,
            no_calls,
            no_methods,
            prereq,
        }
    }

    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;

        let is_setup = meta.prereq_done(self.prereq)?;

        if is_setup && !self.force {
            anyhow::bail!("graph database already setup");
        }

        let source_dir = ctx.get_smali_dir()?.join("framework");
        let db = get_default_graphdb(&ctx)?;

        if !is_setup {
            db.initialize()?;
        }

        let (mon, chan) = ChannelEventMonitor::create();
        let (source_tx, source_rx) = bounded(0);

        let apk_paths = self.get_apk_paths(&ctx)?;
        let num_dirs = apk_paths.len() + 1;
        let _join = start_print_thread(String::from("System Services"), source_rx, chan, num_dirs);

        let (cancel, check) = TaskCanceller::new();
        let opts = AddDirectoryOptions::new(String::from(FRAMEWORK_SOURCE), source_dir)
            .set_no_calls(self.no_calls)
            .set_no_methods(self.no_methods);

        match db.add_directory(&ctx, opts, &mon, &check) {
            Err(e) => return Err(e.into()),
            Ok(()) => {}
        }

        for path in apk_paths {
            self.handle_apk_dir(&ctx, &mon, &path, &check, &source_tx, &db)?;
        }
        drop(mon);
        drop(cancel);
        meta.update_prereq(self.prereq, true)?;
        if matches!(self.prereq, Prereq::GraphDatabaseFullSetup) {
            meta.update_prereq(Prereq::GraphDatabasePartialSetup, true)?;
        }
        Ok(())
    }

    fn get_apk_paths(&self, ctx: &dyn Context) -> anyhow::Result<Vec<PathBuf>> {
        let deny = match self.apk_denylist_file.as_ref() {
            None => None,
            Some(pb) => Some(Denylist::from_path(pb, |s| String::from(s))?),
        };

        let smali_apks = ctx.get_smali_dir()?.join("apks");

        let rd = fs::read_dir(&smali_apks)?;

        let dirs = rd
            .filter(|r| r.as_ref().map_or(false, |e| e.path().is_dir()))
            .map(|r| r.unwrap().path())
            .filter(|it| {
                let pathname = path_must_name(it);
                !opt_deny(&deny, pathname)
            })
            .collect::<Vec<PathBuf>>();

        Ok(dirs)
    }

    fn handle_apk_dir(
        &self,
        ctx: &dyn Context,
        mon: &dyn EventMonitor<SetupEvent>,
        pb: &PathBuf,
        check: &TaskCancelCheck,
        source_tx: &Sender<String>,
        db: &dyn GraphDatabase,
    ) -> anyhow::Result<()> {
        let path = DevicePath::from_path(pb)?;
        source_tx
            .send(String::from(path.device_file_name()))
            .unwrap();
        log::info!("Adding APK {}", path);
        let opts = AddDirectoryOptions::new(path.get_squashed_string(), pb.clone())
            .set_no_calls(self.no_calls)
            .set_no_methods(self.no_methods);
        db.add_directory(ctx, opts, mon, check)?;
        Ok(())
    }
}
