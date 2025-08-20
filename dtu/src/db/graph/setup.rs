use core::fmt;
use std::path::PathBuf;

use crate::db::graph::Error;
use crate::db::sql;
use crate::prereqs::Prereq;
use crate::smalisa_wrapper::write_analysis_files;
use crate::tasks::{EventMonitor, TaskCancelCheck};
use crate::utils::{fs, path_must_str};
use crate::{smalisa_wrapper, Context};
use dtu_proc_macro::define_setters;
use dtu_proc_macro::wraps_base_error;
use walkdir::DirEntry;

use super::GraphDatabase;

pub enum SetupEvent {
    Wiping,
    /// Fired when starting Smalisa
    SmalisaStart {
        total_files: usize,
    },
    /// Fired when Smalisa starts a given file
    SmalisaFileStarted {
        path: String,
    },
    /// Fired when Smalisa completes a given file
    SmalisaFileComplete {
        path: String,
        success: bool,
    },
    /// Fired when all files have been passed through Smalisa
    SmalisaDone,
    /// Fired when importing is started for a given directory
    AllImportsStarted {
        dir: String,
    },
    /// Fired when importing is started for a specific CSV file
    ImportStarted {
        path: String,
    },
    /// Fired when a CSV file is done importing
    ImportDone {
        path: String,
    },
    /// Fired when all CSV files are imported
    AllImportsDone {
        dir: String,
    },
}

#[define_setters]
pub struct AddDirectoryOptions {
    /// Name corresponds to the "source" value in the database
    pub name: String,
    /// The directory containing the files to be added
    pub source_dir: PathBuf,
    /// Don't import methods to the database
    pub no_methods: bool,

    /// Don't import calls to the database
    pub no_calls: bool,
}

impl AddDirectoryOptions {
    pub fn new(name: String, source_dir: PathBuf) -> Self {
        Self {
            name,
            source_dir,
            no_calls: false,
            no_methods: false,
        }
    }
}

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("pull and decompile required for setup")]
    NoPullAndDecompile,
    #[error("already setup")]
    AlreadySetup,
    #[error("database error {0}")]
    SQL(sql::Error),
    #[error("graph error {0}")]
    Neo4j(Error),
    #[error("invalid source directory")]
    InvalidSource,
    #[error("{0}")]
    Smalisa(smalisa_wrapper::Error),
    #[error("user cancelled")]
    Cancelled,
}

impl From<SetupError> for Error {
    fn from(value: SetupError) -> Self {
        match value {
            SetupError::NoPullAndDecompile => {
                Self::Base(crate::Error::UnsatisfiedPrereq(Prereq::PullAndDecompile))
            }
            SetupError::Cancelled => Self::Cancelled,
            _ => Self::Generic(value.to_string()),
        }
    }
}

impl From<smalisa_wrapper::Error> for SetupError {
    fn from(value: smalisa_wrapper::Error) -> Self {
        Self::Smalisa(value)
    }
}

impl From<sql::Error> for SetupError {
    fn from(value: sql::Error) -> Self {
        Self::SQL(value)
    }
}

impl From<Error> for SetupError {
    fn from(value: Error) -> Self {
        Self::Neo4j(value)
    }
}

fn smalisa_class_ignore_func(class: &str) -> bool {
    const IGNORE_LIST: &[&'static str] = &[
        "Landroidx/",
        "Landroid/support/",
        "Landroid/material/",
        "Lkotlin/",
    ];
    for it in IGNORE_LIST {
        if class.starts_with(*it) {
            log::trace!("ignoring class {} due to ignore list entry {}", class, *it);
            return true;
        }
    }
    false
}

fn smalisa_file_ignore_func(ent: &DirEntry) -> bool {
    const IGNORE_LIST: &[&'static str] = &[
        "androidx/",
        "android/support/",
        "android/material/",
        "kotlin/",
        "javax/",
    ];

    let path = ent.path();
    let as_str = match path.to_str() {
        None => return false,
        Some(s) => s,
    };
    for it in IGNORE_LIST {
        if as_str.contains(*it) {
            log::trace!("ignoring file {} due to ignore list entry {}", as_str, *it);
            return true;
        }
    }
    false
}

pub type SetupResult<T> = Result<T, SetupError>;

pub(crate) struct AddDirTask<'a, M, G>
where
    M: EventMonitor<SetupEvent> + ?Sized,
    G: GraphDatabaseInternal,
{
    pub(crate) ctx: &'a dyn Context,
    pub(crate) monitor: &'a M,
    pub(crate) graph: &'a G,
    pub(crate) opts: AddDirectoryOptions,
    pub(crate) cancel: &'a TaskCancelCheck,
}

impl<'a, M, G> AddDirTask<'a, M, G>
where
    M: EventMonitor<SetupEvent> + ?Sized,
    G: GraphDatabaseInternal,
{
    pub(crate) fn run(&self) -> SetupResult<()> {
        self.smalisa_and_import_directory()?;
        Ok(())
    }

    pub fn smalisa_and_import_directory(&self) -> SetupResult<()> {
        log::info!(
            "running smalisa for source {} from directory {}",
            self.opts.name,
            path_must_str(&self.opts.source_dir)
        );
        self.check_cancel()?;
        if !self.import_files_exist()? {
            write_analysis_files(
                self.monitor,
                &self.cancel,
                &self.opts.source_dir,
                &self.get_import_dir()?,
                smalisa_file_ignore_func,
                smalisa_class_ignore_func,
            )?;
            self.monitor.on_event(SetupEvent::SmalisaDone);
        } else {
            log::debug!("skipped smalisa since all import files exist");
        }
        self.check_cancel()?;
        log::info!("importing smalisa generated csvs");
        self.import_smalisa_files()
    }

    fn get_import_dir(&self) -> SetupResult<PathBuf> {
        Ok(self.ctx.get_graph_import_dir()?.join(&self.opts.name))
    }

    fn import_files_exist(&self) -> SetupResult<bool> {
        let dir = self.get_import_dir()?;
        let file_names = &[
            "classes.csv",
            "supers.csv",
            "interfaces.csv",
            "methods.csv",
            "calls.csv",
        ];
        for f in file_names {
            let path = dir.join(f);
            if !path.exists() {
                return Ok(false);
            }
        }
        Ok(true)
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
            .load_classes_csv(self.ctx, &csv, &self.opts.name)?;
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
            .load_supers_csv(self.ctx, &csv, &self.opts.name)?;
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
        self.graph.load_impls_csv(self.ctx, &csv, &self.opts.name)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_methods(&self, import_dir_name: &str) -> SetupResult<()> {
        if self.opts.no_methods {
            return Ok(());
        }
        let csv = format!("{}{}methods.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Methods) {
            return Ok(());
        }
        log::info!("Adding methods...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph
            .load_methods_csv(self.ctx, &csv, &self.opts.name)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_calls(&self, import_dir_name: &str) -> SetupResult<()> {
        if self.opts.no_calls {
            return Ok(());
        }
        let csv = format!("{}{}calls.csv", import_dir_name, fs::OS_PATH_SEP);
        if !self.should_load(LoadCSVKind::Calls) {
            return Ok(());
        }
        log::info!("Adding calls...");
        self.monitor
            .on_event(SetupEvent::ImportStarted { path: csv.clone() });
        self.graph.load_calls_csv(self.ctx, &csv, &self.opts.name)?;
        self.monitor.on_event(SetupEvent::ImportDone { path: csv });
        Ok(())
    }

    fn import_smalisa_files(&self) -> SetupResult<()> {
        let import_dir_name = String::from(path_must_str(&self.get_import_dir()?));

        self.monitor.on_event(SetupEvent::AllImportsStarted {
            dir: import_dir_name.clone(),
        });

        let res = self.do_smalisa_imports(&import_dir_name);

        self.monitor.on_event(SetupEvent::AllImportsDone {
            dir: import_dir_name.clone(),
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
pub(crate) enum LoadCSVKind {
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

pub(crate) trait GraphDatabaseInternal: GraphDatabase {
    fn should_load_csv(&self, source: &str, csv: LoadCSVKind) -> bool;

    fn load_classes_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> super::Result<()>;
    fn load_supers_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> super::Result<()>;
    fn load_impls_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> super::Result<()>;
    fn load_methods_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> super::Result<()>;
    fn load_calls_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> super::Result<()>;
}
