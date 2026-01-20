use std::path::PathBuf;

use crate::graph::monitor::{start_import_print_thread, start_smalisa_print_thread};
use crate::parsers::DevicePathValueParser;
use crate::utils::task_canceller;
use clap::{self, Args};
use crossbeam::channel::bounded;
use dtu::db::graph::AddDirectoryOptions;
use dtu::db::graph::{get_default_graphdb, GraphDatabaseSetup};
use dtu::db::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::tasks::{smalisa, ChannelEventMonitor};
use dtu::utils::DevicePath;
use dtu::{Context, DefaultContext};

#[derive(Args)]
pub struct ImportApkMethods {
    /// The APK to import
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: DevicePath,
}

impl ImportApkMethods {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::GraphDatabaseSetup)?;
        let squashed = self.apk.as_squashed_str();
        let apk_dir = ctx.get_smali_dir()?.join("apks").join(squashed);

        self.smalisa_dir(&ctx, &apk_dir)?;
        self.import_smalisa(&ctx, &apk_dir)
    }

    fn smalisa_dir(&self, ctx: &dyn Context, apk_dir: &PathBuf) -> anyhow::Result<()> {
        let (cancel, check) = task_canceller()?;
        let (mon, chan) = ChannelEventMonitor::create();
        let (_source_tx, source_rx) = bounded(0);

        let _handle =
            start_smalisa_print_thread(self.apk.device_file_name().into(), source_rx, chan, 1);
        let opts = smalisa::AddDirectoryOptions::new(self.apk.get_squashed_string(), apk_dir);
        smalisa::AddDirTask::new(ctx, &mon, opts, &check).run()?;

        drop(mon);
        drop(cancel);
        Ok(())
    }

    fn import_smalisa(&self, ctx: &dyn Context, apk_dir: &PathBuf) -> anyhow::Result<()> {
        let db = get_default_graphdb(&ctx)?;
        let (cancel, check) = task_canceller()?;
        let (mon, chan) = ChannelEventMonitor::create();

        let _handle = start_import_print_thread(self.apk.device_file_name().into(), chan, 1);
        let opts = AddDirectoryOptions::new(self.apk.get_squashed_string(), apk_dir);
        db.add_directory(&ctx, opts, &mon, &check)?;
        drop(mon);
        drop(cancel);
        Ok(())
    }
}
