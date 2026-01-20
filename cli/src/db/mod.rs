mod add_apk;
mod add_service_impl;
mod diff_source;
mod emulator_diff;
mod monitor;
mod setup;

use std::path::PathBuf;

use anyhow::bail;
use clap::{self, Args, Subcommand};

use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::filestore::get_filestore;
use dtu::prereqs::Prereq;
use dtu::utils::ensure_dir_exists;
use dtu::{Context, DefaultContext};

use add_service_impl::AddServiceImpl;
use diff_source::DiffSource;
use emulator_diff::EmulatorDiff;
use setup::Setup;

#[derive(Args)]
pub struct DB {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Setup the database, required for all other commands
    #[command()]
    Setup(Setup),

    /// Create the emulator diff
    EmulatorDiff(EmulatorDiff),

    /// Commands for dealing with diff sources
    #[command()]
    DiffSource(DiffSource),

    /// Add a system service implementation that was not discovered automatically
    ///
    /// This will wipe some information related to the service from the database
    /// and then replace it with new data. This should be used in cases where
    /// the automatic database setup for some reason missed a service.
    #[command()]
    AddServiceImpl(AddServiceImpl),

    /// Manually add an APK to the database
    #[command()]
    AddApk(add_apk::AddApk),

    /// Wipes the whole database
    #[command()]
    Wipe,
}

impl DB {
    pub fn run(&self) -> anyhow::Result<()> {
        match &self.command {
            Commands::AddServiceImpl(c) => c.run(),
            Commands::Setup(c) => c.run(),
            Commands::DiffSource(c) => c.run(),
            Commands::EmulatorDiff(c) => c.run(),
            Commands::AddApk(c) => c.run(),
            Commands::Wipe => self.wipe_database(),
        }
    }

    fn wipe_database(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        let db = DeviceSqliteDatabase::new(&ctx)?;
        db.wipe()?;
        meta.update_prereq(Prereq::EmulatorDiff, false)?;
        meta.update_prereq(Prereq::SQLDatabaseSetup, false)?;
        Ok(())
    }
}

pub(crate) fn get_aosp_database_path(ctx: &dyn Context, api_level: u32) -> anyhow::Result<PathBuf> {
    Ok(ctx
        .get_user_local_dir()?
        .join("aosp")
        .join(api_level.to_string())
        .join("device.db"))
}

pub(crate) fn get_aosp_database(
    ctx: &dyn Context,
    api_level: u32,
) -> anyhow::Result<DeviceSqliteDatabase> {
    let path = get_aosp_database_path(ctx, api_level)?;
    let path_as_str = path.to_str().expect("valid paths");

    if path.exists() {
        return Ok(DeviceSqliteDatabase::new_from_path(path_as_str)?);
    }

    let remote_path = format!("aosp/{}/device.db", api_level);

    let store = get_filestore(ctx)?;

    store.get_file(ctx, &remote_path, path_as_str)?;

    if !path.exists() {
        bail!(
            "failed to copy remote path {} to path {} with store {}",
            remote_path,
            path_as_str,
            store.name(),
        );
    }
    Ok(DeviceSqliteDatabase::new_from_path(path_as_str)?)
}

pub(crate) fn get_path_for_diff_source(ctx: &dyn Context, source: &str) -> anyhow::Result<PathBuf> {
    let diff_source_db_dir = ctx.get_sqlite_dir()?.join("diff_sources");
    ensure_dir_exists(&diff_source_db_dir)?;
    Ok(diff_source_db_dir.join(source).with_extension("db"))
}
