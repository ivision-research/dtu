use std::cmp;
use std::collections::{HashSet, HashMap};
use std::fs;
use std::fs::{read_dir, DirEntry};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;

use crossbeam::channel::{bounded, Receiver, Sender};
use crossbeam::thread::{self, ScopedJoinHandle};
use rayon::{ThreadPool, ThreadPoolBuilder};
use walkdir::WalkDir;

use dtu_proc_macro::{wraps_base_error, wraps_decompile_error};

use crate::db::meta::models::{DecompileStatus, InsertDecompileStatus, ProgressStep};
use crate::db::{self, MetaDatabase};
use crate::decompile::{decompile_file, ApexFile, ApkFile, Decompile, FrameworkFileType};
use crate::devicefs::FindType;
use crate::prereqs::Prereq;
use crate::tasks::{cancelable_recv, cancelable_send, EventMonitor, TaskCancelCheck};
use crate::utils::{
    ensure_dir_exists, path_has_ext, path_must_name, path_must_str, DevicePath, OS_PATH_SEP,
};
use crate::{run_cmd, Context, Error as BaseError};

use crate::devicefs::DeviceFSHelper;

const NUM_HELPER_THREADS: usize = 2;

pub struct Options {
    /// The maximum number of worker threads to use for pulling
    pub worker_threads: usize,

    /// Try VDex files
    ///
    /// VDex files used to contain dexs and could be used for decompilation, but
    /// they don't on later versions
    pub try_vdex: bool,

    /// Force pulling and decompiling even for files that have already been
    /// successfully completed.
    ///
    /// NOT IMPLEMENTED
    pub force: bool,

    /// Retry failed items
    pub retry: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            worker_threads: Options::MIN_WORKER_THREADS,
            try_vdex: false,
            force: false,
            retry: true,
        }
    }
}

#[wraps_base_error]
#[wraps_decompile_error]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to install {0} as apktool framework")]
    InstallApktoolFramework(String),

    #[error("{0}")]
    DBError(db::Error),

    #[error("invalid path")]
    InvalidPath,
}

impl From<db::Error> for Error {
    fn from(value: db::Error) -> Self {
        Self::DBError(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl Options {
    pub const MIN_WORKER_THREADS: usize = 2;

    pub fn from_context(ctx: &dyn Context) -> Self {
        let mut def = Self::default();
        // Dex files inside of vdex became optional for Android 10 (api level 29)
        def.try_vdex = ctx.get_target_api_level() < 29;
        def
    }

    #[inline]
    fn get_worker_threads_count(&self) -> usize {
        usize::max(Options::MIN_WORKER_THREADS, self.worker_threads)
    }
}

/// PullEvents represent different events during the pulling process
pub enum Event {
    FrameworkStarted,
    FrameworkEnded,
    ApksStarted,
    ApksEnded,

    FindingDirectories,
    DirectoryFound { device: DevicePath },
    DirectoryDone { device: DevicePath },

    Pulling { device: DevicePath, local: PathBuf },
    PullSuccess { device: DevicePath },
    PullFailed { device: DevicePath },

    Decompiling { local: PathBuf },
    DecompileSuccess { local: PathBuf },
    DecompileFailed { local: PathBuf },
}

impl Event {
    fn pulling<D, P>(device: D, local: P) -> Self
    where
        D: Into<DevicePath> + ?Sized,
        P: Into<PathBuf> + ?Sized,
    {
        let device = device.into();
        let local = local.into();
        Self::Pulling { device, local }
    }

    fn pulling_done<D>(device: D, success: bool) -> Self
    where
        D: Into<DevicePath> + ?Sized,
    {
        let device = device.into();
        if success {
            Self::PullSuccess { device }
        } else {
            Self::PullFailed { device }
        }
    }

    fn dir_found(device: &str) -> Self {
        let device = DevicePath::new(device);
        Self::DirectoryFound { device }
    }

    fn dir_done(device: &str) -> Self {
        let device = DevicePath::new(device);
        Self::DirectoryDone { device }
    }

    fn decompile_start<P>(local: P) -> Self
    where
        P: Into<PathBuf> + ?Sized,
    {
        let local = local.into();
        Self::Decompiling { local }
    }

    fn decompile_done<P>(local: P, success: bool) -> Self
    where
        P: Into<PathBuf> + ?Sized,
    {
        let local = local.into();
        if success {
            Self::DecompileSuccess { local }
        } else {
            Self::DecompileFailed { local }
        }
    }
}

/// Use adb to pull and decompile all relevant framework files.
pub fn pull(
    ctx: &dyn Context,
    opts: &Options,
    dfs: &dyn DeviceFSHelper,
    meta_db: &dyn MetaDatabase,
    monitor: &dyn EventMonitor<Event>,
    cancel: TaskCancelCheck,
) -> Result<()> {
    let _ = ctx.get_env("DTU_PROJECT_HOME")?;
    // Ensure we have the executables before moving forward
    let _ = ctx.get_bin("baksmali")?;
    let _ = ctx.get_bin("apktool")?;

    log::trace!("starting pull");
    let prog = meta_db.get_progress(Prereq::PullAndDecompile)?;

    if prog.completed && !(opts.force || opts.retry) {
        log::info!("pull already completed");
        return Ok(());
    }

    ensure_cache_dir_tag(ctx)?;

    let helper_pool = ThreadPoolBuilder::new()
        .num_threads(NUM_HELPER_THREADS)
        .build()
        .expect("failed to build helper thread pool");

    let worker_pool = ThreadPoolBuilder::new()
        .num_threads(opts.get_worker_threads_count())
        .build()
        .expect("failed to build worker thread pool");

    let p = Pull {
        ctx,
        dfs,
        monitor,
        meta_db,
        opts,
        cancel: &cancel,
        worker_pool,
        helper_pool,
    };

    p.run()
}

struct Pull<'a> {
    ctx: &'a dyn Context,
    dfs: &'a dyn DeviceFSHelper,
    monitor: &'a dyn EventMonitor<Event>,

    meta_db: &'a dyn MetaDatabase,

    opts: &'a Options,
    cancel: &'a TaskCancelCheck,

    worker_pool: ThreadPool,
    helper_pool: ThreadPool,
}

impl<'a> Pull<'a> {
    fn cancelled(&self) -> bool {
        self.cancel.was_cancelled()
    }

    fn run(&self) -> Result<()> {
        if self.cancelled() {
            return Err(BaseError::Cancelled.into());
        }
        let res = thread::scope(|scope| {
            scope
                .spawn(|scope| {
                    let framework_handle = scope.spawn(|_| self.pull_framework());

                    let mut res: Result<()> =
                        framework_handle.join().expect("failed to join handle");

                    if let Err(e) = &res {
                        log::error!("failed to pull frameworks: {}", e);
                        return res;
                    }

                    if self.cancelled() {
                        log::debug!("user cancelled");
                        return match res {
                            Ok(_) => Err(BaseError::Cancelled.into()),
                            Err(e) => Err(e),
                        };
                    }

                    if res.is_ok() {
                        let apk_handle = scope.spawn(|_| self.pull_apks());
                        res = apk_handle.join().expect("failed to join handle");
                    }

                    res
                })
                .join()
                .expect("failed to join scope")
        })
        .expect("failed to create inner scope");

        if res.is_ok() {
            let prog = ProgressStep {
                step: Prereq::PullAndDecompile,
                completed: true,
            };
            self.meta_db.update_progress(&prog)?;
        }

        res
    }

    fn pull_apks(&self) -> Result<()> {
        log::trace!("pulling apks");
        let smali_dir = self.ctx.get_smali_dir()?.join("apks");
        let apks_dir = self.ctx.get_apks_dir()?;

        let apktool_output_dir = apks_dir.join("decompiled");

        ensure_dir_exists(&smali_dir)?;
        ensure_dir_exists(&apktool_output_dir)?;

        let nthreads = self.opts.get_worker_threads_count();

        let (apk_tx, apk_rx) = bounded(nthreads * 2);

        log::trace!("pulling APKs with {} threads", nthreads);

        thread::scope(|scope| {
            let find_handle = scope.spawn(move |_| {
                if let Err(e) = self.find_apks(apk_tx) {
                    log::error!("failure searching for APKs: {}", e);
                    return Err(e);
                }
                Ok(())
            });

            self.worker_pool.broadcast(|_| loop {
                if let Ok(rcv) = cancelable_recv(self.cancel, &apk_rx) {
                    match rcv {
                        Some(apk) => {
                            log::trace!("decompiling apk: {}", apk);
                            // TODO
                            let _ = self.pull_and_decompile_apk(
                                DevicePath::new(apk),
                                &apks_dir,
                                &apktool_output_dir,
                                &smali_dir,
                            );
                        }
                        None => break,
                    }
                } else {
                    break;
                }
            });

            find_handle.join().expect("joining apk handle")?;
            Ok::<(), Error>(())
        })
        .expect("making scope")?;

        Ok(())
    }

    fn update_decompile_status(&self, status: &DecompileStatus) -> Result<()> {
        self.meta_db.update_decompile_status(status)?;
        Ok(())
    }

    fn create_new_decompile_status(&self, device_path: &DevicePath) -> Result<DecompileStatus> {
        let ins = InsertDecompileStatus::new(device_path.clone(), false, 0);
        let id = self.meta_db.add_decompile_status(&ins)?;
        Ok(DecompileStatus {
            id,
            device_path: ins.device_path,
            host_path: None,
            decompiled: ins.decompiled,
            decompile_attempts: ins.decompile_attempts,
        })
    }

    fn get_path_decompile_status(&self, device_path: &DevicePath) -> Result<DecompileStatus> {
        let status = match self
            .meta_db
            .get_decompile_status_by_device_path(device_path.as_ref())
        {
            Ok(e) => e,
            Err(db::Error::NotFound) => self.create_new_decompile_status(device_path)?,
            Err(e) => return Err(Error::DBError(e)),
        };
        Ok(status)
    }

    fn pull_framework(&self) -> Result<()> {
        log::trace!("pulling frameworks");
        let mut smali_dir = self.ctx.get_smali_dir()?;
        smali_dir.push("framework");
        let apktool_output_dir = self.ctx.get_apks_dir()?.join("framework");
        let frameworks_dir = self.ctx.get_frameworks_dir()?;

        ensure_dir_exists(&frameworks_dir)?;
        ensure_dir_exists(&smali_dir)?;
        ensure_dir_exists(&apktool_output_dir)?;

        let (apks, map) = thread::scope(|scope| self.find_applicable_framework_files(scope))
            .expect("failed to make scope")?;

        if apks.len() > 0 {
            self.worker_pool.scope(|s| {
                for apk in apks {
                    s.spawn(|_| {
                        if self.cancelled() {
                            return;
                        }
                        if let Err(e) = self.pull_and_decompile_framework_apk(
                            DevicePath::from(apk),
                            &frameworks_dir,
                            &apktool_output_dir,
                            &smali_dir,
                        ) {
                            log::error!("handling framework apk {}", e);
                        }
                    })
                }
                Ok::<(), Error>(())
            })?;
        }
        let (tx, rx) =
            bounded::<Vec<NormalizedDeviceFile>>(self.opts.get_worker_threads_count() * 2);

        self.helper_pool.scope(|s| {
            // Single thread to drain the map
            s.spawn(|_| {
                for (_, files) in map {
                    match cancelable_send(self.cancel, files, &tx) {
                        Ok(sent) if sent => {}
                        _ => break,
                    }
                }
                drop(tx);
            });

            // Workers receiving map objects
            self.worker_pool.broadcast(|_| loop {
                let files = match cancelable_recv(self.cancel, &rx) {
                    Ok(Some(it)) => it,
                    _ => break,
                };
                if let Err(e) = self.handle_normalized_fileset(&files, &frameworks_dir, &smali_dir)
                {
                    log::error!("handling framework fileset - {}", e);
                }
            });
        });

        log::trace!("Done pulling frameworks");

        Ok(())
    }

    fn pull_file(&self, device_path: &DevicePath, host_path: &str) -> Result<()> {
        self.send_event(Event::pulling(device_path, host_path));

        match self.dfs.pull(device_path, &host_path) {
            Err(e) => {
                log::error!("pulling {}: {}", device_path, e);
                self.send_event(Event::pulling_done(device_path, false));
                Err(e.into())
            }
            Ok(_) => {
                self.send_event(Event::pulling_done(device_path, true));
                Ok(())
            }
        }
    }

    fn pull_and_decompile_framework_apk(
        &self,
        device_path: DevicePath,
        apks_dir: &PathBuf,
        apktool_output_dir: &PathBuf,
        smali_dir: &PathBuf,
    ) -> Result<()> {
        let mut status = self.get_path_decompile_status(&device_path)?;
        if !status.should_decompile() {
            return Ok(());
        }
        let success = self
            .do_pull_and_decompile_framework_apk(
                device_path,
                &apks_dir,
                &apktool_output_dir,
                &smali_dir,
                &mut status,
            )
            .unwrap_or(false);
        status.decompiled = success;
        status.decompile_attempts += 1;
        self.update_decompile_status(&status)
    }

    fn do_pull_and_decompile_framework_apk(
        &self,
        device_path: DevicePath,
        apks_dir: &PathBuf,
        apktool_output_dir: &PathBuf,
        smali_dir: &PathBuf,
        status: &mut DecompileStatus,
    ) -> Result<bool> {
        let host_path = apks_dir.join(&device_path).to_string_lossy().into_owned();

        if status.host_path.is_none() {
            status.host_path = Some(host_path.clone());
        }

        if status.should_pull() {
            self.pull_file(&device_path, &host_path)?;
        }

        let fd_pb = self.get_frameworks_path().unwrap();
        let fd = fd_pb.to_string_lossy();

        log::trace!("installing {} as an apktool framework", device_path);
        let apktool = self.ctx.get_bin("apktool")?;
        run_cmd(apktool, &["if", &host_path, "-p", &fd])?;

        self.decompile_apk(device_path, host_path, apktool_output_dir, smali_dir)
    }

    fn pull_and_decompile_apk(
        &self,
        device_path: DevicePath,
        apks_dir: &PathBuf,
        apktool_output_dir: &PathBuf,
        smali_dir: &PathBuf,
    ) -> Result<bool> {
        let mut status = self.get_path_decompile_status(&device_path)?;
        if status.decompiled {
            log::debug!("skipping {} due to previous success", device_path);
            return Ok(true);
        }

        if !status.should_decompile() {
            log::debug!("skipping {} due to previous failures", device_path);
            return Ok(false);
        }
        if status.host_path.is_none() {
            let host_fs_path = apks_dir.join(&device_path).to_string_lossy().to_string();
            status.host_path = Some(host_fs_path.clone());
        }

        let host_path = status.host_path.as_ref().map(|x| x.clone()).unwrap();

        if status.should_pull() {
            if let Err(e) = self.pull_file(&device_path, &host_path) {
                status.decompile_attempts += 1;
                let _ = self.update_decompile_status(&status);
                return Err(e);
            }
        }

        let res = self.decompile_apk(device_path, host_path, apktool_output_dir, smali_dir);
        status.decompile_attempts += 1;
        status.decompiled = match res.as_ref() {
            Ok(success) => *success,
            Err(_) => false,
        };
        let decomp_res = self.update_decompile_status(&status);
        if res.is_ok() {
            decomp_res.map(|_| res.unwrap())
        } else {
            res
        }
    }

    fn decompile_apk(
        &self,
        device_path: DevicePath,
        host_path: String,
        apktool_output_dir: &PathBuf,
        smali_dir: &PathBuf,
    ) -> Result<bool> {
        self.send_event(Event::decompile_start(&host_path));

        let frameworks_path_pb = self.get_frameworks_path().unwrap();
        let frameworks_path = frameworks_path_pb.to_string_lossy();

        let apk_file = ApkFile::new(&host_path)
            .set_force(true)
            .set_frameworks_path(&frameworks_path);

        let out_dir = apktool_output_dir.join(&device_path);

        match apk_file.decompile(self.ctx, self.dfs, &out_dir) {
            Err(e) => {
                log::error!("decompiling {}: {:?}", host_path, e);
                self.send_event(Event::decompile_done(host_path.as_str(), false));
                Err(e.into())
            }
            Ok(success) => {
                self.send_event(Event::decompile_done(host_path.as_str(), success));
                if success {
                    let apk_smali_dir = smali_dir.join(&device_path);
                    move_apk_smali(&out_dir, &apk_smali_dir)?;
                }
                Ok(success)
            }
        }
    }

    fn send_event(&self, evt: Event) {
        self.monitor.on_event(evt)
    }

    fn do_decompile_file(
        &self,
        host_path: &String,
        device_path: &DevicePath,
        smali_dir: &PathBuf,
        should_pull: bool,
    ) -> Result<bool> {
        if should_pull {
            self.pull_file(device_path, host_path)?;
        }

        self.send_event(Event::decompile_start(host_path));
        ensure_dir_exists(smali_dir)?;

        if device_path.extension().map_or(false, |it| it == "apex") {
            log::trace!("decompiling an apex file: {}", device_path);
            return self.do_decompile_apex_file(host_path, smali_dir);
        }

        match decompile_file(self.ctx, self.dfs, host_path, smali_dir) {
            Err(e) => {
                self.send_event(Event::decompile_done(host_path.as_str(), false));
                Err(e.into())
            }
            Ok(success) => {
                self.send_event(Event::decompile_done(host_path.as_str(), success));
                Ok(success)
            }
        }
    }

    fn do_decompile_apex_file(&self, host_path: &String, smali_dir: &PathBuf) -> Result<bool> {
        let mut apex = ApexFile::new(host_path);
        // Ensure apex APKs end up decompiled to the same locaton as all other APKs
        let apk_out = self.ctx.get_apks_dir()?.join("decompiled");
        apex.set_apk_output_dir(Some(&apk_out));

        let apk_smali_dir = self.ctx.get_smali_dir()?.join("apks");
        ensure_dir_exists(&apk_smali_dir)?;

        apex.set_apk_output_callback(Some(Box::new(move |apk, output_dir| {
            let new_dir = apk_smali_dir.join(apk);
            if let Err(e) = move_apk_smali(output_dir, &new_dir) {
                log::error!(
                    "moving apk artifacts for {}: {}",
                    path_must_str(output_dir),
                    e,
                );
            }
        })));
        match apex.decompile(&self.ctx, self.dfs, smali_dir) {
            Err(e) => {
                self.send_event(Event::decompile_done(host_path.as_str(), false));
                Err(e.into())
            }
            Ok(success) => {
                self.send_event(Event::decompile_done(host_path.as_str(), success));
                Ok(success)
            }
        }
    }

    fn handle_normalized_fileset(
        &self,
        files: &Vec<NormalizedDeviceFile>,
        frameworks_dir: &PathBuf,
        smali_dir: &PathBuf,
    ) -> Result<()> {
        let mut res: Result<()> = Ok(());
        if files.len() > 1 {
            for nf in files {
                match self.do_decompile_normalized_file(&nf, &frameworks_dir, &smali_dir) {
                    Ok(success) => {
                        if success {
                            res = Ok(());
                            break;
                        } else {
                            continue;
                        }
                    }
                    Err(Error::Base(BaseError::Cancelled)) => {
                        res = Err(Error::Base(BaseError::Cancelled));
                        break;
                    }
                    Err(e) => {
                        res = Err(e);
                        continue;
                    }
                }
            }
        } else {
            let nf = &files[0];
            res = self
                .do_decompile_normalized_file(&nf, &frameworks_dir, &smali_dir)
                .map(|_| ());
        }
        res
    }

    fn do_decompile_normalized_file(
        &self,
        nf: &NormalizedDeviceFile,
        dest_dir: &PathBuf,
        smali_dir: &PathBuf,
    ) -> Result<bool> {
        if !self.opts.try_vdex && nf.file_type == FrameworkFileType::VDex {
            log::debug!("Skipping VDex file");
            return Ok(false);
        }

        let device_path = &nf.device_path;

        let mut status = self.get_path_decompile_status(device_path)?;

        if status.decompiled {
            log::debug!("skipping {} due to previous success", nf.device_path);
            return Ok(true);
        }

        if !status.should_decompile() {
            log::debug!("skipping {} due to previous failures", nf.device_path);
            return Ok(false);
        }

        if status.host_path.is_none() {
            let host_fs_path = dest_dir
                .join(nf.local_fs_name())
                .to_string_lossy()
                .to_string();
            status.host_path = Some(host_fs_path.clone());
        }

        let host_fs_path = status.host_path.as_ref().unwrap();

        let res =
            self.do_decompile_file(&host_fs_path, device_path, smali_dir, status.should_pull());

        status.decompile_attempts += 1;
        status.decompiled = match res.as_ref() {
            Ok(success) => *success,
            Err(_) => false,
        };
        let decomp_res = self.update_decompile_status(&status);
        if res.is_ok() {
            decomp_res.map(|_| res.unwrap())
        } else {
            res
        }
    }

    fn find_applicable_framework_files<'scope, 'env>(
        &'env self,
        scope: &'scope thread::Scope<'env>,
    ) -> Result<(Vec<DevicePath>, HashMap<String, Vec<NormalizedDeviceFile>>)> {
        log::trace!("populating the file map");
        let (dir_tx, dir_rx): (Sender<String>, Receiver<String>) = bounded(4);
        let (file_tx, file_rx): (Sender<NormalizedDeviceFile>, Receiver<NormalizedDeviceFile>) =
            bounded(16);

        self.send_event(Event::FindingDirectories);

        log::trace!("starting dir thread");
        let dir_thread = self.start_dir_thread(scope, dir_rx, file_tx);
        log::trace!("starting file thread");
        let file_thread = self.start_file_thread(scope, file_rx);

        self.find_framework_dirs(dir_tx)?;

        dir_thread.join().expect("failed to join dir thread")?;

        let (apks, map) = file_thread.join().expect("failed to join file thread")?;
        log::trace!(
            "have hash map with {} entries and {} apks",
            map.len(),
            apks.len()
        );

        Ok((apks, map))
    }

    fn start_file_thread<'scope, 'env>(
        &'env self,
        scope: &'scope thread::Scope<'env>,
        file_rx: Receiver<NormalizedDeviceFile>,
    ) -> ScopedJoinHandle<
        'scope,
        Result<(Vec<DevicePath>, HashMap<String, Vec<NormalizedDeviceFile>>)>,
    >
where {
        scope.spawn(move |_| {
            let mut framework_apks = Vec::new();
            let mut file_map: HashMap<String, Vec<NormalizedDeviceFile>> = HashMap::new();

            loop {
                if self.cancelled() {
                    return Err(BaseError::Cancelled.into());
                }
                let val = match cancelable_recv(self.cancel, &file_rx)? {
                    None => break,
                    Some(v) => v,
                };
                on_normed_file_received(&mut framework_apks, &mut file_map, val);
            }

            Ok((framework_apks, file_map))
        })
    }

    fn start_dir_thread<'scope, 'env>(
        &'env self,
        scope: &'scope thread::Scope<'env>,
        dir_rx: Receiver<String>,
        ff_tx: Sender<NormalizedDeviceFile>,
    ) -> ScopedJoinHandle<'scope, Result<()>> {
        scope.spawn(move |_| {
            let mut err: Option<Error> = None;

            loop {
                let dir = match cancelable_recv(self.cancel, &dir_rx)? {
                    None => break,
                    Some(val) => val,
                };
                log::debug!("found dir {}", dir);
                self.send_event(Event::dir_found(&dir));
                let res = self.find_files(&dir, &ff_tx);
                self.send_event(Event::dir_done(&dir));
                match res {
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                    _ => {}
                }
            }

            match err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        })
    }

    fn find_files(&self, dir: &str, ff_tx: &Sender<NormalizedDeviceFile>) -> Result<()> {
        let mut on_stdout = |line: &str| {
            if line.is_empty() {
                return Ok(());
            }
            let nf = match NormalizedDeviceFile::from_device_path(line, String::from(dir)) {
                Some(nf) => nf,
                None => return Ok(()),
            };
            // Only want the framework APKs
            if nf.file_type == FrameworkFileType::Apk && !nf.fname.contains("framework") {
                return Ok(());
            }
            cancelable_send(self.cancel, nf, &ff_tx)?;
            Ok(())
        };

        self.dfs
            .find(dir, FindType::Any, None, None, &mut on_stdout)?;
        //let cmd = format!(
        //    "find '{}' -type f -print0 2> /dev/null",
        //    dir.replace('\'', "'\"'\"'")
        //);
        //self.adb.streamed_find_no_stderr(&cmd, &mut on_stdout)?;
        Ok(())
    }

    fn find_framework_dirs(&self, tx: Sender<String>) -> Result<()> {
        let mut on_dir = |line: &str| {
            if line.is_empty() {
                Ok(())
            } else {
                log::trace!("found framework dir: {}", line);
                cancelable_send(self.cancel, line.into(), &tx)?;
                Ok(())
            }
        };

        self.dfs.find_framework_dirs(&mut on_dir)?;

        //self.adb.streamed_find_no_stderr(
        //    "find / -mindepth 2 -maxdepth 4 -type d -name 'framework' -print0 2> /dev/null",
        //    &mut on_stdout,
        //)?;

        //self.adb.streamed_find_no_stderr(
        //    "find / -mindepth 2 -maxdepth 4 -type d -name 'apex' -print0 2> /dev/null",
        //    &mut on_stdout,
        //)?;

        Ok(())
    }

    /// Find all APKs via `pm package list -f`. This is likely the most
    /// comprehensive way to search for APKs.
    fn find_apks(&self, tx: Sender<String>) -> Result<()> {
        let mut seen: HashSet<String> = HashSet::new();

        let mut on_line = |apk_path: &str| -> anyhow::Result<()> {
            if apk_path.is_empty() || seen.contains(apk_path) {
                return Ok(());
            }

            let apk_path_owned = String::from(apk_path);
            seen.insert(apk_path_owned.clone());
            cancelable_send(self.cancel, apk_path_owned, &tx)?;
            Ok(())
        };

        self.dfs.find_apks(&mut on_line)?;

        Ok(())
    }

    fn get_frameworks_path(&self) -> Result<PathBuf> {
        let path = self.ctx.get_output_dir_child("apktool-frameworks")?;
        ensure_dir_exists(&path)?;
        Ok(path)
    }
}

fn on_normed_file_received(
    apks: &mut Vec<DevicePath>,
    file_map: &mut HashMap<String, Vec<NormalizedDeviceFile>>,
    normed: NormalizedDeviceFile,
) {
    let name = normed.get_normalized_name();

    if normed.file_type == FrameworkFileType::Apk {
        apks.push(normed.device_path);
        return;
    }

    if file_map.contains_key(&name) {
        let v = file_map.get_mut(&name).unwrap();
        v.push(normed);
        v.sort();
    } else {
        let mut v = Vec::with_capacity(2);
        v.push(normed);
        file_map.insert(name, v);
    }
}

/// Move all smali files from the given output directory to the smali_dir. This
/// will handle all smali[_classesN] directories in the directory.
///
/// This maintains the tree structure of the directory
pub fn move_apk_smali(apktool_output_dir: &PathBuf, smali_dir: &PathBuf) -> Result<()> {
    log::trace!(
        "moving APK artifacts from {:?} to {:?}",
        apktool_output_dir,
        smali_dir
    );
    ensure_dir_exists(smali_dir)?;

    // Find all of the `smali[_classesN]` dirs in the apktool output
    let smali_dirs = read_dir(apktool_output_dir)?.filter(|rd| match &rd {
        Ok(e) => {
            let path = e.path();
            if !path.is_dir() {
                return false;
            }
            let name = path_must_name(&path);
            name.starts_with("smali") && !name.starts_with("smali_assets")
        }
        Err(_) => false,
    });

    // Now everything from these smali dirs has to be moved to the smali dir
    for d in smali_dirs {
        move_apk_smali_files(&d.unwrap(), smali_dir)?;
    }
    Ok(())
}

/// Walks the directory and uses fs::rename to move .smali files to the smali_dir.
///
/// This maintains the tree structure of the directory
fn move_apk_smali_files(dent: &DirEntry, smali_dir: &PathBuf) -> Result<()> {
    log::trace!("Moving all smali files from {:?} to {:?}", dent, smali_dir);

    let parent = dent.path();
    let walk = WalkDir::new(&parent)
        .into_iter()
        .filter(|rd| {
            rd.as_ref().map_or(false, |e| {
                let path = e.path();
                path.is_file() && path_has_ext(path, "smali")
            })
        })
        .map(|rd| rd.unwrap());
    let parent_str = parent.to_str().ok_or(Error::InvalidPath)?;
    for ent in walk {
        let path = ent.path();
        let file_str = path.to_str().ok_or(Error::InvalidPath)?;
        let new_path_str = file_str
            .strip_prefix(parent_str)
            .ok_or(Error::InvalidPath)?
            .trim_start_matches(OS_PATH_SEP);
        let new_path = smali_dir.join(new_path_str);
        if let Some(p) = new_path.parent() {
            ensure_dir_exists(p)?;
        }
        fs::rename(path, &new_path)?;
    }
    Ok(())
}

/// NormalizedDeviceFile is intended to deal with the fact that there are
/// multiple versions of the same file on the device. We don't want to
/// pull and decompile the same thing multiple times, so we try to normalize
/// them.
#[cfg_attr(debug_assertions, derive(Debug, Clone))]
struct NormalizedDeviceFile {
    device_path: DevicePath,
    base_dir: String,
    fname: String,
    file_type: FrameworkFileType,
}

impl NormalizedDeviceFile {
    fn from_device_path(device_path_str: &str, base_dir: String) -> Option<Self> {
        let device_path = DevicePath::new(device_path_str);
        let file_type = FrameworkFileType::from_device_path(&device_path)?;

        let fname = device_path.device_file_name();

        let normed_fname = match fname.strip_prefix("boot-") {
            None => fname,
            Some(s) => s,
        };

        // Removing the suffix so foo.jar and foo.vdex are treated the same
        let (fname, _suffix) = normed_fname.rsplit_once('.')?;

        let fname = String::from(fname);

        Some(Self {
            device_path,
            base_dir,
            fname,
            file_type,
        })
    }

    fn local_fs_name(&self) -> &str {
        self.device_path.as_squashed_str()
    }

    fn get_normalized_name(&self) -> String {
        if self.base_dir.ends_with('/') {
            format!("{}{}", self.base_dir, self.fname)
        } else {
            format!("{}/{}", self.base_dir, self.fname)
        }
    }
}

impl PartialEq for NormalizedDeviceFile {
    fn eq(&self, other: &Self) -> bool {
        self.file_type == other.file_type
            && self.fname == other.fname
            && self.base_dir == other.base_dir
    }
}

impl Eq for NormalizedDeviceFile {}

impl PartialOrd for NormalizedDeviceFile {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NormalizedDeviceFile {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.file_type.cmp(&other.file_type)
    }
}

impl Hash for NormalizedDeviceFile {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fname.hash(state);
        self.file_type.hash(state);
        self.base_dir.hash(state);
    }
}

fn ensure_cache_dir_tag(ctx: &dyn Context) -> Result<()> {
    let base_dir = ctx.get_output_dir()?;
    ensure_dir_exists(&base_dir)?;
    let cache_dir = base_dir.join("CACHEDIR.TAG");
    if cache_dir.exists() {
        return Ok(());
    }

    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&cache_dir)?;

    let content = r#"Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by dtu.
# For information about cache directory tags, see:
#	http://www.brynosaurus.com/cachedir/
"#;

    f.write_all(content.as_bytes())?;

    Ok(())
}

#[cfg(test)]
mod test {
    use std::collections::hash_map::DefaultHasher;
    use std::fs::OpenOptions;
    use std::hash::Hasher;
    use std::io::{Read, Write};

    use super::*;
    use crate::testing::{tmp_context, TestContext};
    use rstest::*;

    #[rstest]
    fn test_move_apk_smali(tmp_context: TestContext) {
        let proj_home = tmp_context.get_project_dir().expect("project dir");

        let from_dir = proj_home.join("from").join("%system%app%test.apk");
        let to_dir = proj_home.join("to").join("%system%app%test.apk");

        let smali_classes2 = from_dir.join("smali_classes2");
        let smali = from_dir.join("smali");
        let junk = from_dir.join("junk");

        let junk_file = PathBuf::from("junk_file");
        let baz_smali = PathBuf::from("Test.smali");
        let foo_smali = PathBuf::from("com").join("foo").join("Test.smali");
        let quux_smali = PathBuf::from("com").join("quux").join("Test.smali");
        let bar_smali = PathBuf::from("com").join("bar").join("Test.smali");

        macro_rules! write_file {
            ($fname:expr, $into:ident) => {{
                let path = $into.join($fname);
                let parent = path.parent().unwrap();
                ensure_dir_exists(&parent).unwrap();
                let mut f = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .open(&path)
                    .unwrap();
                write!(&mut f, "blah blah").unwrap();
                drop(f);
            }};
        }

        macro_rules! check_file {
            ($fname:expr) => {{
                let path = to_dir.join($fname);
                assert!(path.exists(), "path {:?} doesn't exist", path);
                let mut f = OpenOptions::new().read(true).open(&path).unwrap();
                let mut into = String::new();
                f.read_to_string(&mut into).expect("reading file");
                assert_eq!(into.as_str(), "blah blah", "contents didn't match");
            }};
        }

        write_file!(&junk_file, junk);
        write_file!(&baz_smali, smali_classes2);
        write_file!(&foo_smali, smali_classes2);
        write_file!(&bar_smali, smali);
        write_file!(&quux_smali, smali);

        ensure_dir_exists(&smali).expect("mkdir");
        ensure_dir_exists(&junk).expect("mkdir");
        ensure_dir_exists(&smali_classes2).expect("mkdir");

        move_apk_smali(&from_dir, &to_dir).expect("move_apk_smali");

        check_file!(&baz_smali);
        check_file!(&bar_smali);
        check_file!(&foo_smali);
        check_file!(&quux_smali);

        let junk = to_dir.join("junk_file");
        assert_eq!(junk.exists(), false, "shouldn't have moved the junk file");
    }

    #[test]
    fn test_normalized_device_file() {
        let base_sf = String::from("/system/framework");
        let base_pf = String::from("/product/framework");

        let sboot = String::from("/system/framework/boot-foo.jar");
        let snorm = String::from("/system/framework/foo.jar");

        let boot = NormalizedDeviceFile::from_device_path(&sboot, base_sf.clone())
            .expect("should parse boot");
        let norm = NormalizedDeviceFile::from_device_path(&snorm, base_sf.clone())
            .expect("should parse norm");

        assert_eq!(boot, norm, "normalized files not equal");

        assert_eq!(
            boot.local_fs_name(),
            "%system%framework%boot-foo.jar",
            "local fs names not equal"
        );

        let mut hasher = DefaultHasher::new();
        boot.hash(&mut hasher);
        let hv1 = hasher.finish();
        hasher = DefaultHasher::new();
        norm.hash(&mut hasher);
        let hv2 = hasher.finish();

        assert_eq!(hv1, hv2, "hashes should be equal");

        let sboot = String::from("/system/framework/arm/boot-foo.jar");
        let snorm = String::from("/system/framework/foo.jar");

        let boot = NormalizedDeviceFile::from_device_path(&sboot, base_sf.clone())
            .expect("should parse boot");
        let norm = NormalizedDeviceFile::from_device_path(&snorm, base_sf.clone())
            .expect("should parse norm");

        assert_eq!(boot, norm, "normalized files not equal");

        let mut hasher = DefaultHasher::new();
        boot.hash(&mut hasher);
        let hv1 = hasher.finish();
        hasher = DefaultHasher::new();
        norm.hash(&mut hasher);
        let hv2 = hasher.finish();

        assert_eq!(hv1, hv2, "hashes should be equal");

        let sboot = String::from("/system/framework/bar.jar");
        let snorm = String::from("/system/framework/foo.jar");

        let boot = NormalizedDeviceFile::from_device_path(&sboot, base_sf.clone())
            .expect("should parse boot");
        let norm = NormalizedDeviceFile::from_device_path(&snorm, base_sf.clone())
            .expect("should parse norm");

        assert_ne!(boot, norm, "normalized files shouldn't be equal");

        let sboot = String::from("/product/framework/foo.vdex");
        let snorm = String::from("/system/framework/foo.vdex");

        let boot = NormalizedDeviceFile::from_device_path(&sboot, base_pf.clone())
            .expect("should parse boot");
        let norm = NormalizedDeviceFile::from_device_path(&snorm, base_sf.clone())
            .expect("should parse norm");

        assert_ne!(boot, norm, "normalized files shouldn't be equal");

        let sboot = String::from("/product/framework/boot-foo.vdex");
        let snorm = String::from("/system/framework/foo.vdex");

        let boot = NormalizedDeviceFile::from_device_path(&sboot, base_pf.clone())
            .expect("should parse boot");
        let norm = NormalizedDeviceFile::from_device_path(&snorm, base_sf.clone())
            .expect("should parse norm");

        assert_ne!(boot, norm, "normalized files shouldn't be equal");
    }

    #[test]
    fn test_on_normed_file_received() {
        let mut apks = Vec::new();
        let mut file_map = HashMap::new();
        let normed_apk = NormalizedDeviceFile::from_device_path(
            "/system/framework/test.apk",
            "/system/framework".into(),
        )
        .unwrap();
        let normed_non_apk = NormalizedDeviceFile::from_device_path(
            "/system/framework/test.jar",
            "/system/framework".into(),
        )
        .unwrap();
        let normed_non_apk2 = NormalizedDeviceFile::from_device_path(
            "/system/framework/test.vdex",
            "/system/framework".into(),
        )
        .unwrap();

        let normed_non_apk_diff = NormalizedDeviceFile::from_device_path(
            "/system/framework/foo.jar",
            "/system/framework".into(),
        )
        .unwrap();

        on_normed_file_received(&mut apks, &mut file_map, normed_apk.clone());
        assert_eq!(apks.len(), 1);
        assert_eq!(apks.get(0), Some(&normed_apk.device_path));
        on_normed_file_received(&mut apks, &mut file_map, normed_non_apk.clone());
        assert_eq!(file_map.len(), 1);
        assert_eq!(
            file_map.get(&normed_non_apk.get_normalized_name()),
            Some(&vec![normed_non_apk.clone()])
        );
        on_normed_file_received(&mut apks, &mut file_map, normed_non_apk2.clone());
        assert_eq!(file_map.len(), 1);
        assert_eq!(
            file_map.get(&normed_non_apk.get_normalized_name()),
            Some(&vec![normed_non_apk.clone(), normed_non_apk2.clone()])
        );
        on_normed_file_received(&mut apks, &mut file_map, normed_non_apk_diff.clone());
        assert_eq!(file_map.len(), 2);
    }
}
