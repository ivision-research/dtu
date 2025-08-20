use crate::db::sql::{self, MetaDatabase};
use crate::devicefs::{DeviceFSHelper, FindName, FindType};
use crate::prereqs::Prereq;
use crate::tasks::EventMonitor;
use crate::utils::{ensure_dir_exists, path_must_str, DevicePath};
use crate::{run_cmd, Context};
use dtu_proc_macro::wraps_base_error;
use regex::Regex;
use std::path::PathBuf;

pub struct Options {
    pub force: bool,
}

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    DBError(sql::Error),
}

impl From<sql::Error> for Error {
    fn from(value: sql::Error) -> Self {
        Self::DBError(value)
    }
}

pub enum Event {
    SearchingForPolicyFiles,
    FoundPolicyFiles { files: Vec<DevicePath> },
    PullingFile { file: DevicePath },
    BuildingPolicy,
    Done,
}

pub type Result<T> = std::result::Result<T, Error>;

struct PullTask<'a> {
    ctx: &'a dyn Context,
    mon: &'a dyn EventMonitor<Event>,
    meta: &'a dyn MetaDatabase,
    dfs: &'a dyn DeviceFSHelper,
    opts: &'a Options,
}

pub fn pull(
    ctx: &dyn Context,
    meta: &dyn MetaDatabase,
    mon: &dyn EventMonitor<Event>,
    dfs: &dyn DeviceFSHelper,
    opts: &Options,
) -> Result<()> {
    let task = PullTask {
        ctx,
        mon,
        meta,
        opts,
        dfs,
    };

    task.run()
}

impl<'a> PullTask<'a> {
    fn run(&self) -> Result<()> {
        if self.meta.prereq_done(Prereq::AcquiredSelinuxPolicy)? && !self.opts.force {
            return Err(Error::Base(crate::Error::TaskAlreadyDone));
        }

        let base_dir = self.ctx.get_selinux_dir()?;
        ensure_dir_exists(&base_dir)?;

        let secilc = self.ctx.get_bin("secilc")?;

        let files = self.get_files()?;

        let policy_out = base_dir.join("policy.33");

        let mut args = vec![
            "-m",
            "-M",
            "true",
            "-G",
            "-N",
            "-o",
            path_must_str(&policy_out),
        ];

        for f in &files {
            args.push(path_must_str(f));
        }

        self.emit(Event::BuildingPolicy);

        run_cmd(&secilc, args.as_slice())?;

        self.emit(Event::Done);

        self.meta
            .update_prereq(Prereq::AcquiredSelinuxPolicy, true)?;

        Ok(())
    }

    fn get_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let out_dir = self.ctx.get_selinux_dir()?.join("cils");
        ensure_dir_exists(&out_dir)?;
        let paths = self.find_files()?;
        for p in paths {
            let out_path = out_dir.join(p.as_squashed_str());
            self.emit(Event::PullingFile { file: p.clone() });
            self.dfs.pull(&p, path_must_str(&out_path))?;
            files.push(out_path);
        }
        Ok(files)
    }

    fn find_files(&self) -> Result<Vec<DevicePath>> {
        let mut paths = Vec::new();
        let re = Regex::new(r"[0-9]+\.[0-9]\.cil").unwrap();

        self.emit(Event::SearchingForPolicyFiles);

        let mut on_file = |line: &str| {
            let line = line.trim();
            if line.len() == 0 || line.ends_with(".compat.cil") || re.find(line).is_some() {
                return Ok(());
            }
            log::trace!("found file: {}", line);
            paths.push(DevicePath::new(line));
            Ok(())
        };

        self.dfs.find(
            "/",
            FindType::File,
            None,
            Some(FindName::Suffix(".cil")),
            &mut on_file,
        )?;

        //self.adb.streamed_find_no_stderr(
        //    "find / -type f -name '*.cil' -print0 2> /dev/null",
        //    &mut on_file,
        //)?;

        self.emit(Event::FoundPolicyFiles {
            files: paths.clone(),
        });

        Ok(paths)
    }

    fn emit(&self, evt: Event) {
        self.mon.on_event(evt);
    }
}
