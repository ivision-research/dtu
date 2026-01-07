use core::fmt;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::db::graph::Error;
use crate::db::sql;
use crate::tasks::{EventMonitor, TaskCancelCheck};
use crate::utils::{fs, opt_deny, path_must_name, path_must_str, DevicePath, OptDenylist};
use crate::Context;
use dtu_proc_macro::wraps_base_error;

use super::GraphDatabase;

pub enum SetupEvent {
    Wiping,

    SourceStarted {
        source: String,
    },

    /// Fired when all CSV files are imported for the given source
    SourceDone {
        source: String,
    },

    /// Fired when importing is started for a specific CSV file
    ImportStarted {
        path: String,
    },
    /// Fired when a CSV file is done importing
    ImportDone {
        path: String,
    },
}

pub struct AddDirectoryOptions<'a> {
    /// Name corresponds to the "source" value in the database
    pub name: String,
    /// The directory containing the Smalisa CSV files to be added
    pub source_dir: Cow<'a, Path>,
}

impl<'a> AddDirectoryOptions<'a> {
    pub fn new<T: Into<Cow<'a, Path>>>(name: String, source_dir: T) -> Self {
        Self {
            name,
            source_dir: source_dir.into(),
        }
    }
}

pub struct InitialImportOptions {
    pub apk_denylist: OptDenylist<DevicePath>,
}

impl Default for InitialImportOptions {
    fn default() -> Self {
        Self::new(None)
    }
}

impl InitialImportOptions {
    pub fn new(apk_denylist: OptDenylist<DevicePath>) -> Self {
        Self { apk_denylist }
    }

    #[inline]
    pub fn allows_apk(&self, path: &DevicePath) -> bool {
        !opt_deny(&self.apk_denylist, path)
    }

    /// Get the smalisa directories for each APK respecting the denylist
    pub fn get_apk_smalisa_dirs(&self, ctx: &dyn Context) -> crate::Result<Vec<PathBuf>> {
        let smali_apks = ctx.get_smali_dir()?.join("apks");

        let rd = std::fs::read_dir(&smali_apks)?;

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
                if opt_deny(&self.apk_denylist, &squashed) {
                    return None;
                }
                Some(path)
            })
            .collect::<Vec<PathBuf>>();

        Ok(dirs)
    }
}

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("already setup")]
    AlreadySetup,
    #[error("graph error {0}")]
    Graph(Error),
    #[error("database error {0}")]
    SQL(sql::Error),
    #[error("invalid source directory")]
    InvalidSource,
    #[error("{0}")]
    Generic(String),
    #[error("user cancelled")]
    Cancelled,
}

impl From<SetupError> for Error {
    fn from(value: SetupError) -> Self {
        match value {
            SetupError::Cancelled => Self::Cancelled,
            _ => Self::Generic(value.to_string()),
        }
    }
}

impl From<sql::Error> for SetupError {
    fn from(value: sql::Error) -> Self {
        Self::SQL(value)
    }
}

impl From<Error> for SetupError {
    fn from(value: Error) -> Self {
        Self::Graph(value)
    }
}

pub type SetupResult<T> = Result<T, SetupError>;

pub struct AddDirTask<'a, M, G>
where
    M: EventMonitor<SetupEvent> + ?Sized,
    G: GraphDatabaseSetup,
{
    pub(crate) ctx: &'a dyn Context,
    pub(crate) monitor: &'a M,
    pub(crate) graph: &'a G,
    pub(crate) opts: AddDirectoryOptions<'a>,
    pub(crate) cancel: &'a TaskCancelCheck,
}

impl<'a, M, G> AddDirTask<'a, M, G>
where
    M: EventMonitor<SetupEvent> + ?Sized,
    G: GraphDatabaseSetup,
{
    pub fn new(
        ctx: &'a dyn Context,
        monitor: &'a M,
        graph: &'a G,
        opts: AddDirectoryOptions<'a>,
        cancel: &'a TaskCancelCheck,
    ) -> Self {
        Self {
            ctx,
            monitor,
            graph,
            opts,
            cancel,
        }
    }

    pub fn run(&self) -> SetupResult<()> {
        log::info!("importing smalisa generated csvs");
        self.import_smalisa_files()
    }

    fn get_import_dir(&self) -> SetupResult<PathBuf> {
        Ok(self.ctx.get_graph_import_dir()?.join(&self.opts.name))
    }

    fn should_load(&self, kind: LoadCSVKind) -> bool {
        self.graph.should_load_csv(&self.opts.name, kind)
    }

    fn import_classes(&self, import_dir_name: &str) -> SetupResult<()> {
        let csv = format!("{}{}classes.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Classes) {
            return Ok(());
        }
        log::info!("Adding classes...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph
            .load_csv(self.ctx, &csv, &self.opts.name, LoadCSVKind::Classes)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_supers(&self, import_dir_name: &str) -> SetupResult<()> {
        let csv = format!("{}{}supers.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Supers) {
            return Ok(());
        }
        log::info!("Adding supers...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph
            .load_csv(self.ctx, &csv, &self.opts.name, LoadCSVKind::Supers)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_interfaces(&self, import_dir_name: &str) -> SetupResult<()> {
        let csv = format!("{}{}interfaces.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Impls) {
            return Ok(());
        }
        log::info!("Adding interfaces...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph
            .load_csv(self.ctx, &csv, &self.opts.name, LoadCSVKind::Impls)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_methods(&self, import_dir_name: &str) -> SetupResult<()> {
        let csv = format!("{}{}methods.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Methods) {
            return Ok(());
        }
        log::info!("Adding methods...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph
            .load_csv(self.ctx, &csv, &self.opts.name, LoadCSVKind::Methods)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_calls(&self, import_dir_name: &str) -> SetupResult<()> {
        let csv = format!("{}{}calls.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Calls) {
            return Ok(());
        }
        log::info!("Adding calls...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph
            .load_csv(self.ctx, &csv, &self.opts.name, LoadCSVKind::Calls)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_smalisa_files(&self) -> SetupResult<()> {
        let import_dir_name = String::from(path_must_str(&self.get_import_dir()?));

        self.graph.load_begin(self.ctx)?;

        self.monitor.on_event(SetupEvent::SourceStarted {
            source: self.opts.name.clone(),
        });

        let mut res = self.do_smalisa_imports(&import_dir_name);

        if res.is_ok() {
            res = self
                .graph
                .load_complete(self.ctx, true)
                .map_err(SetupError::from)
        } else {
            _ = self.graph.load_complete(self.ctx, false);
        }

        self.monitor.on_event(SetupEvent::SourceDone {
            source: self.opts.name.clone(),
        });

        res
    }

    fn do_smalisa_imports(&self, import_dir_name: &str) -> SetupResult<()> {
        self.check_cancel()?;
        self.import_classes(import_dir_name)?;

        self.check_cancel()?;
        self.import_interfaces(import_dir_name)?;

        self.check_cancel()?;
        self.import_supers(import_dir_name)?;

        self.check_cancel()?;
        self.import_methods(import_dir_name)?;

        self.check_cancel()?;
        self.import_calls(import_dir_name)?;
        Ok(())
    }

    #[inline]
    fn check_cancel(&self) -> SetupResult<()> {
        self.cancel.check(SetupError::Cancelled)
    }
}

/// Type of CSV file to be loaded
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum LoadCSVKind {
    Classes,
    Supers,
    Impls,
    Methods,
    Calls,
}

impl fmt::Display for LoadCSVKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                LoadCSVKind::Impls => "Impls",
                LoadCSVKind::Classes => "Classes",
                LoadCSVKind::Supers => "Supers",
                LoadCSVKind::Methods => "Methods",
                LoadCSVKind::Calls => "Calls",
            }
        )
    }
}

pub trait GraphDatabaseSetup: Sync + Send + GraphDatabase {
    /// Add all sources known by dtu to the graph database
    ///
    /// This should be preferred to a loop over add_directory for the initial setup so that the
    /// graph database implementation can perform its own setup and finalization steps.
    fn run_initial_import(
        &self,
        ctx: &dyn Context,
        opts: InitialImportOptions,
        monitor: &dyn EventMonitor<SetupEvent>,
        cancel: &TaskCancelCheck,
    ) -> SetupResult<()>;

    /// Adds the contents of a given directory to the graph.
    ///
    /// Use this only for adding a single directory at a time, the graph database must be setup up
    /// initially with [run_initial_import]!
    fn add_directory(
        &self,
        ctx: &dyn Context,
        opts: AddDirectoryOptions<'_>,
        monitor: &dyn EventMonitor<SetupEvent>,
        cancel: &TaskCancelCheck,
    ) -> SetupResult<()>;

    /// Check whether the given CSV kind should be loaded for the given source
    ///
    /// This is used for restarting loads on failures. Ideally, the graph database would
    /// ensure either a given LoadCSVKind is entirely loaded or at all, so that we can
    /// restart after fixing the file manually if needed.
    fn should_load_csv(&self, source: &str, csv: LoadCSVKind) -> bool;

    /// Load the CSV into the database. This should be an all or nothing operation: if the
    /// CSV load fails there should be no partial data from that CSV in the database.
    fn load_csv(
        &self,
        ctx: &dyn Context,
        path: &str,
        source: &str,
        csv: LoadCSVKind,
    ) -> super::Result<()>;

    /// Called when all loading begins
    fn load_begin(&self, ctx: &dyn Context) -> super::Result<()> {
        _ = ctx;
        Ok(())
    }

    /// Called when all loading is completed
    ///
    /// This is called even on failure just in case any cleanup is needed.
    fn load_complete(&self, ctx: &dyn Context, success: bool) -> super::Result<()> {
        _ = ctx;
        _ = success;
        Ok(())
    }
}
