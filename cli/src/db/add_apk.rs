use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::bail;
use clap::{self, Args};
use dtu::{
    db::sql::{
        self,
        device::{
            get_default_devicedb, models::DiffSource, AddApkTask, ApkIdentifier, DiffOptions,
            DiffTask,
        },
        DeviceDatabase, DeviceSqliteDatabase,
    },
    decompile::{ApkFile, Decompile},
    devicefs::{get_project_devicefs_helper, DeviceFSHelper},
    prereqs::Prereq,
    tasks::{pull::move_apk_smali, ChannelEventMonitor, TaskCanceller},
    utils::{ensure_prereq, path_must_str, DevicePath},
    Context, DefaultContext,
};

use crate::{
    db::get_path_for_diff_source,
    utils::{hook_to_signals, task_canceller},
};

use super::{monitor::PrintMonitor, setup::start_monitor_thread};

#[derive(Args)]
pub struct AddApk {
    /// The APK to add
    #[arg(short, long)]
    apk: PathBuf,

    /// Remove all references to the APK in the database and re add it
    ///
    /// Note that this won't redo the decompilation
    #[arg(short, long)]
    force: bool,

    /// The path on the device (ie /system/app/Apk/Apk.apk)
    #[arg(short, long)]
    device_path: String,

    /// Don't show progress output
    #[arg(short, long)]
    quiet: bool,

    /// Don't perform a diff for this APK
    #[arg(long)]
    no_diff: bool,
}

impl AddApk {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;

        let db = get_default_devicedb(&ctx)?;
        let dfs = get_project_devicefs_helper(&ctx)?;
        let device_path = DevicePath::new(&self.device_path);
        let apks_dir = ctx.get_apks_dir()?;
        let apk_file = apks_dir.join(&device_path);
        if !apk_file.exists() {
            fs::copy(&self.apk, &apk_file)?;
        }

        let apktool_out_dir = apks_dir.join("decompiled").join(&device_path);
        let smali_dir = ctx.get_smali_dir()?.join("apks");
        let apk_smali_dir = smali_dir.join(&device_path);

        if !apktool_out_dir.exists() {
            self.decompile(&ctx, &dfs, &apktool_out_dir)?;
            move_apk_smali(&apktool_out_dir, &apk_smali_dir)?;
        }

        let apk_id = match db.get_apk_by_device_path(device_path.as_device_str()) {
            Ok(v) => Some(v.id),
            Err(sql::Error::NotFound) => None,
            Err(e) => return Err(e.into()),
        };

        if let Some(id) = apk_id {
            if !self.force {
                bail!("database already contains APK, use -f/--force to replace it");
            }

            // This should delete everything due to foreign keys and cascades
            db.delete_apk_by_id(id)?;
        }

        let (cancel, check) = TaskCanceller::new();
        let _cancel = hook_to_signals(cancel)?;
        let (mon, chan) = ChannelEventMonitor::create();

        let thread_handle = start_monitor_thread(self.quiet, chan);
        let task = AddApkTask::new(
            &ctx,
            &db,
            Some(&mon),
            &apktool_out_dir,
            &device_path,
            None,
            ApkIdentifier::new(0, 0),
            &check,
        );
        let res = task.run();
        drop(mon);
        let err_reporter = thread_handle
            .join()
            .expect("failed to join thread")
            .unwrap();
        res?;

        if self.no_diff {
            return err_reporter.as_err();
        }

        let existing_diffs = db.get_diff_sources()?;

        for source in existing_diffs {
            self.do_diff(&ctx, source, &db)?;
        }

        err_reporter.as_err()
    }

    fn do_diff(
        &self,
        ctx: &dyn Context,
        source: DiffSource,
        db: &dyn DeviceDatabase,
    ) -> anyhow::Result<()> {
        let diff_db_path = get_path_for_diff_source(ctx, &source.name)?;
        let diff_db = DeviceSqliteDatabase::new_from_path(path_must_str(&diff_db_path))?;
        let (_cancel, check) = task_canceller()?;
        let (mon, _handle) = PrintMonitor::start()?;
        let opts = DiffOptions::new(source);
        let mut task = DiffTask::new(opts, db, &diff_db, check, &mon);
        task.do_system_services = false;
        let res = task.run();
        drop(mon);
        res?;
        Ok(())
    }

    fn decompile(
        &self,
        ctx: &dyn Context,
        dfs: &dyn DeviceFSHelper,
        out_dir: &Path,
    ) -> anyhow::Result<()> {
        let path_str = path_must_str(&self.apk);
        let frame_path = ctx.get_output_dir_child("apktool-frameworks")?;
        let apk_file = ApkFile::new(path_str)
            .set_force(true)
            .set_frameworks_path(path_must_str(&frame_path));

        apk_file.decompile(ctx, dfs, out_dir)?;

        Ok(())
    }
}
