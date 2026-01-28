use std::path::PathBuf;

use anyhow::bail;
use clap::{self, Args};

use dtu::db::device::schema::diff_sources;
use dtu::db::device::{models, DiffOptions, DiffTask, EMULATOR_DIFF_SOURCE};
use dtu::db::{DeviceDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::diesel::insert_into;
use dtu::diesel::prelude::*;
use dtu::prereqs::Prereq;
use dtu::utils::path_must_str;
use dtu::{Context, DefaultContext};

use crate::utils::task_canceller;

use super::get_aosp_database;
use super::monitor::PrintMonitor;

#[derive(Args)]
pub struct EmulatorDiff {
    /// Force the diff to be recreated if it already exists
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    force: bool,

    /// Path to the emulator device.db file if it shouldn't be found
    /// automatically
    #[arg(short, long)]
    path: Option<PathBuf>,

    /// API level for diffing, otherwise taken from device, not
    /// applicable if -p/--path is set.
    #[arg(short = 'A', long)]
    api_level: Option<u32>,
}

impl EmulatorDiff {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        let db = DeviceDatabase::new(&ctx)?;

        let mut prereq = meta.get_progress(Prereq::EmulatorDiff)?;
        if self.force {
            self.wipe_emulator_diff(&db)?;
            if prereq.completed {
                prereq.completed = false;
                meta.update_progress(&prereq)?;
            }
        } else if prereq.completed {
            bail!("emulator already set up and -f not set");
        }

        let other_db = self.get_diff_db(&ctx)?;

        let source = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
        let res = self.add_source(source, &db, &other_db);

        if res.is_ok() {
            prereq.completed = true;
            meta.update_progress(&prereq)?;
        }

        res
    }

    fn get_diff_db(&self, ctx: &dyn Context) -> anyhow::Result<DeviceDatabase> {
        if let Some(p) = &self.path {
            let path_str = path_must_str(p);
            return Ok(DeviceDatabase::new_from_path(path_str)?);
        }

        let api_level = self.api_level.unwrap_or_else(|| ctx.get_target_api_level());
        get_aosp_database(ctx, api_level)
    }

    fn wipe_emulator_diff(&self, db: &DeviceDatabase) -> anyhow::Result<()> {
        let ds = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
        db.delete_diff_source_by_id(ds.id)?;
        let ins = models::InsertDiffSource { name: &ds.name };
        db.with_connection(|c| insert_into(diff_sources::table).values(&ins).execute(c))?;
        Ok(())
    }

    fn add_source(
        &self,
        new_source: models::DiffSource,
        db: &DeviceDatabase,
        other_db: &DeviceDatabase,
    ) -> anyhow::Result<()> {
        let (_cancel, check) = task_canceller()?;
        let (mon, _join) = PrintMonitor::start()?;

        let opts = DiffOptions::new(new_source);
        let task = DiffTask::new(opts, db, other_db, check, &mon);
        let res = task.run();
        drop(mon);
        Ok(res?)
    }
}
