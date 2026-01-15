use std::fs;
use std::path::PathBuf;

use anyhow::bail;
use clap::{self, Args, Subcommand};

use dtu::db::sql::device::{models, DiffOptions, DiffTask};
use dtu::db::sql::{DeviceDatabase, DeviceSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::{ensure_prereq, path_must_str};
use dtu::DefaultContext;

use super::get_path_for_diff_source;
use super::monitor::PrintMonitor;
use crate::parsers::DiffSourceValueParser;
use crate::utils::task_canceller;

#[derive(Args)]
pub struct DiffSource {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args)]
pub struct DB {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new diff source
    #[command()]
    Add(Add),

    /// Remove a diff source
    #[command()]
    Remove(Remove),

    /// List diff sources

    #[command()]
    List(List),
}

#[derive(Args)]
struct Add {
    /// Path to the source's `device.db` file
    #[arg(short, long)]
    path: PathBuf,

    /// Name for the diff source
    #[arg(short, long)]
    name: String,
}

impl Add {
    fn run(&self, db: DeviceSqliteDatabase) -> anyhow::Result<()> {
        let other_db_path = path_must_str(&self.path);
        let other_db = DeviceSqliteDatabase::new_from_path(other_db_path)?;
        let name = self.name.clone();
        let ins = models::InsertDiffSource { name: &name };
        let id = db.add_diff_source(&ins)?;
        let new_source = models::DiffSource { id, name };
        let res = self.add_source(new_source, &db, &other_db);

        if res.is_err() {
            eprintln!("failed to use diff source, removing...");
            match db.delete_diff_source_by_id(id) {
                Ok(_) => log::info!("removed new diff source"),
                Err(e) => log::error!("error removing diff source {}", e),
            }
        } else {
            let ctx = DefaultContext::new();
            let db_path = get_path_for_diff_source(&ctx, &self.name)?;
            if let Err(e) = fs::copy(&self.path, &db_path) {
                bail!(
                    "failed to copy {} to {}: {}",
                    path_must_str(&self.path),
                    path_must_str(&db_path),
                    e
                );
            }
        }

        res
    }

    fn add_source(
        &self,
        new_source: models::DiffSource,
        db: &DeviceSqliteDatabase,
        other_db: &DeviceSqliteDatabase,
    ) -> anyhow::Result<()> {
        let (_cancel, check) = task_canceller()?;
        let (mon, _handle) = PrintMonitor::start()?;
        let opts = DiffOptions::new(new_source);
        let task = DiffTask::new(opts, db, other_db, check, &mon);
        let res = task.run();
        drop(mon);
        Ok(res?)
    }
}

#[derive(Args)]
struct Remove {
    /// Diff source to remove
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    source: models::DiffSource,
}

impl Remove {
    fn run(&self, db: DeviceSqliteDatabase) -> anyhow::Result<()> {
        db.delete_diff_source_by_id(self.source.id)?;
        let ctx = DefaultContext::new();
        let db_path = get_path_for_diff_source(&ctx, &self.source.name)?;
        if db_path.exists() {
            fs::remove_file(&db_path)?;
        }
        Ok(())
    }
}

#[derive(Args)]
struct List {}

impl List {
    fn run(&self, db: DeviceSqliteDatabase) -> anyhow::Result<()> {
        let sources = db.get_diff_sources()?;
        for s in &sources {
            println!("{}", s.name);
        }
        Ok(())
    }
}

impl DiffSource {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;
        let db = DeviceSqliteDatabase::new(&ctx)?;
        match &self.command {
            Commands::Add(c) => c.run(db),
            Commands::Remove(c) => c.run(db),
            Commands::List(c) => c.run(db),
        }
    }
}
