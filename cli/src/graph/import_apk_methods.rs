use super::monitor::start_print_thread;
use crate::parsers::DevicePathValueParser;
use clap::{self, Args};
use crossbeam::channel::bounded;
use dtu::db::graph::AddDirectoryOptions;
use dtu::db::graph::{get_default_graphdb, GraphDatabase};
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::tasks::{ChannelEventMonitor, TaskCanceller};
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
        meta.ensure_prereq(Prereq::GraphDatabasePartialSetup)?;
        let db = get_default_graphdb(&ctx)?;
        let squashed = self.apk.as_squashed_str();

        let apk_dir = ctx.get_smali_dir()?.join("apks").join(squashed);

        let (_cancel, check) = TaskCanceller::new();
        let (mon, chan) = ChannelEventMonitor::create();
        let (_source_tx, source_rx) = bounded(0);

        let _join = start_print_thread(self.apk.device_file_name().into(), source_rx, chan, 1);
        let opts = AddDirectoryOptions::new(self.apk.get_squashed_string(), apk_dir);

        db.add_directory(&ctx, opts, &mon, &check)?;

        Ok(())
    }
}
