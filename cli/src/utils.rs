use std::any::Any;
use std::ffi::CString;
use std::fmt::Display;
use std::fs;
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::JoinHandle;

use anyhow::bail;
use anyhow::Context as AnyhowContext;
use itertools::Itertools;
use promptly::prompt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook::iterator::Handle;
use signal_hook::iterator::Signals;

use dtu::adb::{Adb, ExecAdb};
use dtu::app::server::{ConnectError, TcpAppServer};
use dtu::db::DeviceDatabase;
use dtu::tasks::{TaskCancelCheck, TaskCanceller};
use dtu::utils::{find_file_for_class, find_smali_file_for_class, ClassName, DevicePath};
use dtu::{run_cmd, Context};

/// Wrapper for functions with a per project cacheable result
///
/// If `force` is false, this will look for the result in a project local cache file
/// and use that if it exists, otherwise, the passed function is called to create the
/// object and then written to the cache before being returned.
pub fn project_cacheable<F, R: Serialize + DeserializeOwned>(
    ctx: &dyn Context,
    cache_file: &str,
    force: bool,
    f: F,
) -> anyhow::Result<R>
where
    F: FnOnce() -> anyhow::Result<R>,
{
    let cache_bust = is_cachebust(ctx);
    if cache_bust {
        return f();
    }
    let cache_path = ctx
        .get_project_cache_dir()?
        .join(cache_file)
        .with_extension("postcard");

    if !force && cache_path.exists() {
        let mut f = dtu::utils::fs::open_file(&cache_path)?;
        let mut data = match f.metadata() {
            Ok(v) => {
                let size = v.size();
                Vec::with_capacity(size as usize)
            }
            Err(_) => Vec::with_capacity(1024),
        };
        f.read_to_end(&mut data)
            .with_context(|| format!("reading cache file: {cache_file}"))?;
        return Ok(postcard::from_bytes(&data)?);
    }

    let it = f()?;
    if let Ok(f) = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&cache_path)
    {
        _ = postcard::to_io(&it, BufWriter::new(f));
    }
    Ok(it)
}

pub fn is_cachebust(ctx: &dyn Context) -> bool {
    ctx.has_env("DTU_CACHEBUST")
}

pub fn shash<T: AsRef<str> + ?Sized>(sha: &mut Sha256, s: &T) {
    let as_str = s.as_ref();
    let bytes = as_str.as_bytes();
    sha.update(bytes);
}

pub fn oshash<T: AsRef<str>>(sha: &mut Sha256, opt: &Option<T>) {
    if let Some(s) = opt {
        shash(sha, s.as_ref());
    }
}

/// Convenience function to get an [AppServer] implementation and give a user
/// friendly error.
pub fn get_app_server(ctx: &dyn Context) -> anyhow::Result<TcpAppServer> {
    get_app_server_recur(ctx, false)
}

fn get_app_server_recur(ctx: &dyn Context, called: bool) -> anyhow::Result<TcpAppServer> {
    match TcpAppServer::from_ctx(ctx) {
        Err(e) => match &e {
            ConnectError::ConnectFailed { port, err, .. } => match err.kind() {
                ErrorKind::ConnectionRefused => {
                    if !called {
                        if let Ok(adb) = get_adb(ctx, false) {
                            if let Ok(_) = adb.forward_tcp_port(*port, *port) {
                                return get_app_server_recur(ctx, true);
                            }
                        }
                    }
                    bail!("couldn't reach the server, is port {} forwarded?", port);
                }
                _ => bail!("failed to connect to the app server {}", e),
            },
            _ => bail!("failed to connect to the app server {}", e),
        },
        Ok(v) => return Ok(v),
    }
}

pub fn get_adb_if_configured(
    ctx: &dyn Context,
    prompt_on_multiple: bool,
) -> anyhow::Result<Option<ExecAdb>> {
    let config = ctx.get_project_config()?;
    if let Some(conf) = config {
        let base = conf.get_map();
        let can_adb = base
            .maybe_get_map_typecheck("device-access")?
            .map(|it| it.get_bool_or("can-adb", true))
            .unwrap_or(true);

        if can_adb {
            get_adb(ctx, prompt_on_multiple).map(Some)
        } else {
            Ok(None)
        }
    } else {
        get_adb(ctx, prompt_on_multiple).map(Some)
    }
}

/// Gets an [Adb] implementation using the context
///
/// If multiple devices are plugged in and ANDROID_SERIAL isn't set,
/// the user will be prompted if `prompt_on_multiple` is true. When using this
/// tool, `ANDROID_SERIAL` should basically always be set.
pub fn get_adb(ctx: &dyn Context, prompt_on_multiple: bool) -> anyhow::Result<ExecAdb> {
    let has_serial = ctx.has_env("ANDROID_SERIAL");
    let adb = ExecAdb::new(ctx)?;
    let devices = adb.get_connected_devices()?;
    if devices.is_empty() {
        bail!("no adb device connected");
    }
    let count = devices.len();
    if has_serial || count == 1 {
        return Ok(adb);
    }
    if !prompt_on_multiple {
        bail!("multiple adb devices connected and ANDROID_SERIAL unset");
    }

    let serial = prompt_choice(
        &devices,
        "Multiple ADB devices found, please select one:",
        "Device number: ",
    )?;

    return Ok(ExecAdb::builder(ctx).with_serial(serial.clone()).build());
}

#[cfg(windows)]
pub fn exec_open_file(ctx: &dyn Context, file_name: &str) -> anyhow::Result<()> {
    invoke_dtu_open_file(ctx, file_name, "")
}

#[cfg(not(windows))]
pub fn exec_open_file(ctx: &dyn Context, file_name: &str) -> anyhow::Result<()> {
    let cmd = match ctx.get_env("DTU_EDITOR") {
        Ok(v) => v,
        Err(_) => ctx.get_env("EDITOR")?,
    };

    let cmd_cstr = CString::new(cmd.as_str())?;
    let file_cstr = CString::new(file_name)?;

    let args = &[&cmd_cstr, &file_cstr];

    nix::unistd::execv(&cmd_cstr, args)?;

    panic!("execve failed")
}

/// Given an apk name (Test.apk) find the fully qualified APK name
///
/// We store APKs as squashed paths (@system@priv-app@Test.apk) instead of
/// just Test.apk. This way they're always unique, but these paths aren't
/// very ergonomic, so this function helps find APKs.
pub fn find_fully_qualified_apk(
    ctx: &dyn Context,
    apk_name: &str,
) -> anyhow::Result<Vec<DevicePath>> {
    let dir = ctx.get_apks_dir()?;
    let vals = fs::read_dir(&dir)?
        .filter(|it| {
            it.as_ref()
                .map(|d| {
                    let path = d.path();
                    if !path.is_file() {
                        return false;
                    }
                    let path = DevicePath::from_path(&path);
                    path.map_or(false, |it| it.device_file_name() == apk_name)
                })
                .unwrap_or(false)
        })
        .map(|it| DevicePath::from_path(&it.unwrap().path()).unwrap())
        .collect::<Vec<DevicePath>>();

    Ok(vals)
}

#[allow(dead_code)]
pub fn quoted_args<I, E>(args: I) -> String
where
    E: Display,
    I: IntoIterator<Item = E>,
{
    args.into_iter()
        .map(|e| format!("'{}'", e.to_string().replace("'", "'\"'\"'")))
        .join(" ")
}

pub fn vec_to_single<'a, E: Display>(
    choices: &'a Vec<E>,
    desc: &str,
    prompt_text: &str,
) -> anyhow::Result<&'a E> {
    if choices.len() == 0 {
        bail!("no available choices");
    } else if choices.len() == 1 {
        Ok(choices.get(0).unwrap())
    } else {
        prompt_choice(choices, desc, prompt_text)
    }
}

pub fn prompt_choice<'a, E: Display>(
    choices: &'a Vec<E>,
    desc: &str,
    prompt_text: &str,
) -> anyhow::Result<&'a E> {
    println!("{}", desc);
    let count = choices.len();
    for (i, c) in choices.iter().enumerate() {
        println!("({}) {}", i, c);
    }

    loop {
        let sel: usize = match prompt(prompt_text) {
            Ok(ans) => ans,
            Err(e) => bail!("prompt failed: {}", e),
        };
        if sel >= count {
            eprintln!("invalid selection {}", sel);
            continue;
        }

        return Ok(choices.get(sel).unwrap());
    }
}

pub fn find_smali_file(
    ctx: &dyn Context,
    class: &ClassName,
    apk: &Option<DevicePath>,
    fallback_to_apks: bool,
) -> anyhow::Result<String> {
    let mut smali_file = match find_smali_file_for_class(ctx, class, apk.as_ref()) {
        Some(f) => f,
        None => bail!("couldn't find smali file for {}", class),
    };

    let mut exists = smali_file.exists();

    if !exists {
        if !class.has_pkg() && apk.is_some() {
            log::debug!("trying to use the APK name as the package");
            match try_get_apk_smali_no_pkg(ctx, class, apk) {
                Some(f) => {
                    exists = f.exists();
                    smali_file = f;
                }
                None => {}
            }
        }

        if !exists && fallback_to_apks {
            log::debug!("falling back to checking all APKs");
            // If no apk was provided, just do a last check to see if maybe the file
            // exists in one
            if apk.is_none() {
                if let Some(it) = find_file_for_class(ctx, &class) {
                    smali_file = it;
                    exists = true;
                }
            }
        }
    }

    if !exists {
        bail!("couldn't find file for {}", class);
    }

    let fname = match smali_file.to_str() {
        Some(f) => f,
        None => bail!("bad file name"),
    };
    Ok(fname.into())
}

fn try_get_apk_smali_no_pkg(
    ctx: &dyn Context,
    class: &ClassName,
    apk: &Option<DevicePath>,
) -> Option<PathBuf> {
    let apk = apk.as_ref()?;
    let db = DeviceDatabase::new(ctx).ok()?;
    let apk_name = apk.device_file_name();
    let db_apk = db.get_apk_by_apk_name(apk_name).ok()?;
    let new_class = class.with_new_package(&db_apk.app_name);
    log::debug!("trying to find class {} instead", new_class);
    Some(find_smali_file_for_class(ctx, &new_class, Some(apk))?)
}

/// Invoke $DTU_OPEN_EXECUTABLE or `dtu-open-file` with the given args
///
/// The executable is invoked with `path` as $1 and `search` as $2
pub fn invoke_dtu_open_file(ctx: &dyn Context, path: &str, search: &str) -> anyhow::Result<()> {
    let exe = match ctx.maybe_get_env("DTU_OPEN_EXECUTABLE") {
        Some(v) => v,
        None => match ctx.maybe_get_bin("dtu-open-file") {
            Some(v) => v,
            None => anyhow::bail!(
                "either set DTU_OPEN_EXECUTABLE or add a dtu-open-file executable to $PATH"
            ),
        },
    };
    run_cmd(exe, &[path, search])?.err_on_status()?;
    Ok(())
}

/// Invoke $DTU_CLIPBOARD_EXECUTABLE or `dtu-clipboard` with the passed string
///
/// The clipboard content is sent to the stdin of the process
pub fn invoke_dtu_clipboard(ctx: &dyn Context, content: &str) -> anyhow::Result<()> {
    let exe = match ctx.maybe_get_env("DTU_CLIPBOARD_EXECUTABLE") {
        Some(v) => v,
        None => match ctx.maybe_get_bin("dtu-clipboard") {
            Some(v) => v,
            None => anyhow::bail!(
                "either set DTU_CLIPBOARD_EXECUTABLE or add dtu-clipboard executable to $PATH"
            ),
        },
    };

    let mut cmd = Command::new(&exe)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to run {}: {}", exe, e))?;

    let mut stdin = cmd.stdin.take().unwrap();
    let _ = stdin.write_all(content.as_bytes());
    drop(stdin);
    let status = cmd.wait()?;
    if !status.success() {
        anyhow::bail!("clipboard command ({}) failed", exe);
    }
    Ok(())
}

pub struct HookedSignals {
    sig_handle: Handle,
    _join_handle: JoinHandle<()>,
}

impl Drop for HookedSignals {
    fn drop(&mut self) {
        if !self.sig_handle.is_closed() {
            self.sig_handle.close();
        }
    }
}

pub fn hook_to_signals(mut cancel: TaskCanceller) -> anyhow::Result<HookedSignals> {
    let mut sigs = Signals::new(TERM_SIGNALS)?;
    let sig_handle = sigs.handle();
    let _join_handle = std::thread::spawn(move || {
        let mut seen_exit = false;
        for sig in sigs.forever() {
            if seen_exit {
                _ = signal_hook::low_level::emulate_default_handler(sig);
            } else {
                cancel.cancel();
                seen_exit = true;
            }
        }
    });

    Ok(HookedSignals {
        sig_handle,
        _join_handle,
    })
}

pub fn task_canceller() -> anyhow::Result<(HookedSignals, TaskCancelCheck)> {
    let (cancel, check) = TaskCanceller::new();
    Ok((hook_to_signals(cancel)?, check))
}

// This type can be returned from print threads to ensure that the thread is shutdown and joined but
// it can easily be misused and lead to a deadlock. Essentially, ensure that the `TaskCancelCheck`
// is used within whichever thread this is made from.
pub struct CancelCheckThread<T>
where
    T: Send + 'static,
{
    canceller: TaskCanceller,
    join_handle: Option<JoinHandle<T>>,
}

impl<T> Drop for CancelCheckThread<T>
where
    T: Send + 'static,
{
    fn drop(&mut self) {
        _ = self.shutdown();
    }
}

impl<T> CancelCheckThread<T>
where
    T: Send + 'static,
{
    pub fn spawn<F>(f: F) -> Self
    where
        F: FnOnce(TaskCancelCheck) -> T,
        F: Send + 'static,
    {
        let (canceller, cancel_check) = TaskCanceller::new();
        let join_handle = std::thread::spawn(move || f(cancel_check));

        Self {
            canceller,
            join_handle: Some(join_handle),
        }
    }

    pub fn join(mut self) -> Result<Option<T>, Box<dyn Any + Send + 'static>> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<Option<T>, Box<dyn Any + Send + 'static>> {
        match self.join_handle.take() {
            None => Ok(None),
            Some(v) => {
                self.canceller.cancel();
                v.join().map(|it| Some(it))
            }
        }
    }
}

pub type EmptyCancelCheckThread = CancelCheckThread<()>;

/// Helper to convert things to [Option<&str>]s
pub fn ostr<T: AsRef<str>>(s: &Option<T>) -> Option<&str> {
    s.as_ref().map(|it| it.as_ref())
}
