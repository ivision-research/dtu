use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use walkdir::DirEntry;

use crate::{
    smalisa_wrapper::write_analysis_files,
    tasks::{EventMonitor, TaskCancelCheck},
    utils::path_must_str,
    Context,
};

pub use crate::smalisa_wrapper::{Error, Event};

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

pub type Result<T> = std::result::Result<T, Error>;

pub struct AddDirectoryOptions<'a> {
    /// Name corresponds to the "source" value in the database
    pub name: String,
    /// The directory containing the files to be added
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

pub struct AddDirTask<'a, M>
where
    M: EventMonitor<Event> + ?Sized,
{
    ctx: &'a dyn Context,
    monitor: &'a M,
    opts: AddDirectoryOptions<'a>,
    cancel: &'a TaskCancelCheck,
}

impl<'a, M> AddDirTask<'a, M>
where
    M: EventMonitor<Event> + ?Sized,
{
    pub fn new(
        ctx: &'a dyn Context,
        monitor: &'a M,
        opts: AddDirectoryOptions<'a>,
        cancel: &'a TaskCancelCheck,
    ) -> Self {
        Self {
            ctx,
            monitor,
            opts,
            cancel,
        }
    }

    pub fn run(self) -> Result<()> {
        log::info!(
            "running smalisa for source {} from directory {}",
            self.opts.name,
            path_must_str(&self.opts.source_dir)
        );
        if !self.import_files_exist()? {
            write_analysis_files(
                self.monitor,
                &self.cancel,
                &self.opts.source_dir,
                &self.get_import_dir()?,
                smalisa_file_ignore_func,
                smalisa_class_ignore_func,
            )?;
        } else {
            log::debug!("skipped smalisa since all import files exist");
        }

        Ok(())
    }

    fn get_import_dir(&self) -> Result<PathBuf> {
        Ok(self.ctx.get_graph_import_dir()?.join(&self.opts.name))
    }

    fn import_files_exist(&self) -> Result<bool> {
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
}
