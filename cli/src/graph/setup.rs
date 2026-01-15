use std::fs;
use std::path::PathBuf;

use crate::graph::monitor::{start_import_print_thread, start_smalisa_print_thread};
use crate::utils::task_canceller;

use clap::{self, Args};
use crossbeam::channel::{unbounded, Sender};
use dtu::db::graph::db::FRAMEWORK_SOURCE;
use dtu::db::graph::{get_default_graphdb, GraphDatabaseSetup, InitialImportOptions};
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::tasks::{smalisa, ChannelEventMonitor, EventMonitor, TaskCancelCheck};
use dtu::utils::{opt_deny, path_must_name, Denylist, DevicePath, OptDenylist};
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

    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    only_smalisa: bool,

    /// Denylist of APKs
    ///
    /// Some APKs have _ton_ of classes and maybe shouldn't be included in the database. To this
    /// end, you can specify a denylist file for APKs to be excluded. The file should be newline
    /// separated and `#` are treated as comments.
    ///
    /// To deny an APK, put its device path on a single line of this file.
    #[arg(short = 'l', long)]
    apk_denylist_file: Option<PathBuf>,
}

impl Setup {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;

        let is_setup = meta.prereq_done(Prereq::GraphDatabaseSetup)?;

        if is_setup && !self.force {
            anyhow::bail!("graph database already setup");
        }

        let deny = self.get_denylist()?;

        let apk_paths = self.get_apk_paths(&ctx, &deny)?;
        // +1 for framework
        let num_sources = apk_paths.len() + 1;

        if !meta.prereq_done(Prereq::Smalisa)? {
            self.run_smalisa(&ctx, num_sources, &apk_paths)?;
            meta.update_prereq(Prereq::Smalisa, true)?;
        }

        if self.only_smalisa {
            return Ok(());
        }

        self.run_import(&ctx, num_sources, deny)?;
        meta.update_prereq(Prereq::GraphDatabaseSetup, true)?;
        Ok(())
    }

    fn run_smalisa(
        &self,
        ctx: &dyn Context,
        num_sources: usize,
        apk_paths: &Vec<PathBuf>,
    ) -> anyhow::Result<()> {
        let source_dir = ctx.get_smali_dir()?.join("framework");
        let (mon, chan) = ChannelEventMonitor::create();
        let (source_tx, source_rx) = unbounded();
        let (cancel, check) = task_canceller()?;
        let _handle =
            start_smalisa_print_thread(String::from("framework"), source_rx, chan, num_sources);
        let opts = smalisa::AddDirectoryOptions::new(String::from(FRAMEWORK_SOURCE), source_dir);

        smalisa::AddDirTask::new(ctx, &mon, opts, &check).run()?;

        for path in apk_paths {
            if check.was_cancelled() {
                break;
            }
            self.smalisa_apk_dir(&ctx, &mon, &path, &check, &source_tx)?;
        }

        drop(mon);
        drop(cancel);

        Ok(())
    }

    fn run_import(
        &self,
        ctx: &dyn Context,
        num_sources: usize,
        deny: OptDenylist<DevicePath>,
    ) -> anyhow::Result<()> {
        let db = get_default_graphdb(&ctx)?;

        let (mon, chan) = ChannelEventMonitor::create();
        let (cancel, check) = task_canceller()?;

        let _handle = start_import_print_thread(String::from("framework"), chan, num_sources);
        let opts = InitialImportOptions::new(deny);
        db.run_initial_import(ctx, opts, &mon, &check)?;
        drop(mon);
        drop(cancel);
        Ok(())
    }

    fn get_denylist(&self) -> anyhow::Result<OptDenylist<DevicePath>> {
        // The denylist file will just be a list of device paths to the APK
        Ok(match self.apk_denylist_file.as_ref() {
            None => None,
            Some(pb) => Some(Denylist::from_path(pb, |s| DevicePath::new(s))?),
        })
    }

    fn get_apk_paths(
        &self,
        ctx: &dyn Context,
        deny: &OptDenylist<DevicePath>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let smali_apks = ctx.get_smali_dir()?.join("apks");

        let rd = fs::read_dir(&smali_apks)?;

        let dirs = rd
            .filter_map(|it| {
                let ent = it.ok()?;
                let path = ent.path();
                // Only directories matter
                if !path.is_dir() {
                    return None;
                }
                // Check the squashed path against the denylist
                let squashed = DevicePath::from_squashed(path_must_name(&path));
                if opt_deny(deny, &squashed) {
                    return None;
                }
                Some(path)
            })
            .collect::<Vec<PathBuf>>();

        Ok(dirs)
    }

    fn smalisa_apk_dir(
        &self,
        ctx: &dyn Context,
        mon: &dyn EventMonitor<smalisa::Event>,
        pb: &PathBuf,
        check: &TaskCancelCheck,
        source_tx: &Sender<String>,
    ) -> anyhow::Result<()> {
        let path = DevicePath::from_path(pb)?;
        source_tx
            .send(String::from(path.device_file_name()))
            .unwrap();
        log::info!("Running Smalisa on APK {}", path);
        let opts = smalisa::AddDirectoryOptions::new(path.get_squashed_string(), pb.clone());
        smalisa::AddDirTask::new(ctx, mon, opts, check).run()?;
        Ok(())
    }
}
