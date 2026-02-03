use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::process::{Child, ExitStatus};

use crossbeam::channel::Receiver;
use log;
use log::log_enabled;
use log::Level::Debug;
use regex::Regex;

use crate::command::log_cmd;
use crate::command::{
    run_cmd, run_cmd_split_streamed, run_cmd_streamed, spawn_cmd, CmdOutput, LineCallback,
    OutputCallback,
};
use crate::config::AdbConfig;
use crate::config::DeviceAccessConfig;
use crate::config::ProjectConfig;
use crate::utils::ensure_dir_exists;
use crate::utils::path_must_str;
use crate::Context;

// TODO: I don't think the Adb trait should be so heavily tied to the Command
//  API, but it isn't that big of a deal so whatever.

/// The Adb trait just abstracts some `adb` commands
pub trait Adb: Send + Sync {
    fn get_connected_devices(&self) -> crate::Result<Vec<String>>;

    /// Install the APK at the given path
    fn install(&self, apk: &str) -> crate::Result<()>;

    /// Uninstall the given package
    fn uninstall(&self, package: &str) -> crate::Result<()>;

    /// Essentially the same as running `adb pull $device $local`
    fn pull(&self, device: &str, local: &str) -> io::Result<CmdOutput>;

    /// Similar to `pull`, but returns a handle to the child process instead
    /// of waiting for it to finish
    fn spawn_pull(&self, device: &str, local: &str) -> io::Result<Child>;

    /// Essentially the same as running `adb push $local $device`
    fn push(&self, local: &str, device: &str) -> io::Result<CmdOutput>;

    /// Similar to `push`, but returns a handle to the child process instead
    /// of waiting for it to finish.
    fn spawn_push(&self, local: &str, device: &str) -> io::Result<Child>;

    /// Essentially the same as running `adb shell '...'`
    fn shell(&self, shell_cmd: &str) -> io::Result<CmdOutput>;

    /// Equivalent to `adb backup -all` but places the output file in the [dest] directory
    fn backup(&self, dest: &PathBuf) -> io::Result<()>;

    fn shell_streamed(
        &self,
        shell_cmd: &str,
        on_stdout: &mut OutputCallback,
        on_stderr: &mut OutputCallback,
        kill_child: Option<Receiver<()>>,
    ) -> io::Result<ExitStatus>;

    /// Same as `adb reverse tcp:local_port tcp:remote_port`
    fn reverse_tcp_port(&self, local_port: u16, remote_port: u16) -> io::Result<CmdOutput> {
        let local = format!("tcp:{}", local_port);
        let remote = format!("tcp:{}", remote_port);
        self.reverse_generic(&local, &remote)
    }

    /// Same as `adb forward tcp:local_port tcp:remote_port`
    fn forward_tcp_port(&self, local_port: u16, remote_port: u16) -> io::Result<CmdOutput> {
        let local = format!("tcp:{}", local_port);
        let remote = format!("tcp:{}", remote_port);
        self.forward_generic(&local, &remote)
    }

    /// Same as `adb forward LOCAL REMOTE`
    fn forward_generic(&self, local: &str, remote: &str) -> io::Result<CmdOutput>;

    /// Same as `adb reverse LOCAL REMOTE`
    fn reverse_generic(&self, local: &str, remote: &str) -> io::Result<CmdOutput>;

    /// Same as `shell`, but instead allows for a callback to read the command
    /// output line by line before the process finishes.
    fn shell_split_streamed(
        &self,
        shell_cmd: &str,
        split_on: u8,
        on_stdout_line: &mut LineCallback,
        on_stderr_line: &mut LineCallback,
    ) -> io::Result<ExitStatus>;

    /// Convenience wrapper to stream results of a `find .... 2> /dev/null`
    /// command. If the `2> /dev/null` redirect is not included in the command,
    /// this function will append it.
    ///
    /// This is generally a helpful wrapper because we're often doing vague
    /// `find` invocations that will attempt to search things we don't have
    /// access to.
    fn streamed_find_no_stderr(
        &self,
        find_str: &str,
        on_found: &mut LineCallback,
    ) -> crate::Result<ExitStatus> {
        let mut stderr = String::new();

        let full_find_str = if find_str.contains("2>") {
            Cow::Borrowed(find_str)
        } else {
            Cow::Owned(format!("{} 2> /dev/null", find_str))
        };

        let split_on = if full_find_str.contains("-print0") {
            0u8
        } else {
            b'\n'
        };

        // Since we're using '2> /dev/null' in the shell command, anything output to
        // stderr is actually coming from adb itself.
        let mut on_stderr = |line: &str| {
            if !line.is_empty() {
                stderr.push_str(line);
            }
            Ok(())
        };

        let status =
            self.shell_split_streamed(&full_find_str, split_on, on_found, &mut on_stderr)?;

        if !stderr.is_empty() {
            log::debug!("adb bin stderr: {}", stderr);
            return Err(if stderr.contains("no devices/emulators") {
                crate::Error::NoAdbDevice
            } else {
                let regex = Regex::new(r"device\s+'([^']+)'\s+not\s+found").unwrap();
                let caps = match regex.captures(&stderr) {
                    Some(caps) => caps,
                    None => return Err(crate::Error::Generic(stderr)),
                };
                let serial = caps
                    .get(1)
                    .map(|m| String::from(m.as_str()))
                    .unwrap_or_else(|| "?".into());
                crate::Error::AdbDeviceNotFound(serial)
            });
        }
        Ok(status)
    }
}

#[derive(Clone)]
/// An `Adb` implementation that just invokes the external `adb` command.
pub struct ExecAdb {
    bin: String,
    serial: Option<String>,
}

impl ExecAdb {
    /// Creates a new `ExecAdb` from the given context.
    ///
    /// This will first check the project config file to for device-fs implementation:
    ///
    /// [device-fs]
    /// adb = { serial = "...", executable = "..." }
    ///
    /// and use that if found. If the config file exists and doesn't have that entry, defaults will
    /// be pulled from the environment. `can-adb = false` in the config, this function will fail.
    /// Note that `can-adb` defaults to true.
    pub fn new(ctx: &dyn Context) -> crate::Result<Self> {
        match ctx.get_project_config() {
            Ok(v) => Self::try_from_project_config(ctx, &v),
            Err(_) => Self::from_env(ctx),
        }
    }

    pub fn from_env(ctx: &dyn Context) -> crate::Result<Self> {
        let bin = ctx.get_bin("adb")?;
        let serial = ctx.maybe_get_env("ANDROID_SERIAL");

        Ok(Self { bin, serial })
    }

    pub fn has_serial(&self) -> bool {
        self.serial.is_some()
    }

    pub fn with_serial(mut self, serial: String) -> Self {
        self.serial = Some(serial);
        self
    }

    fn try_from_project_config(ctx: &dyn Context, cfg: &ProjectConfig) -> crate::Result<Self> {
        if !cfg.can_adb {
            return Err(crate::Error::AdbDisabled);
        }
        match &cfg.device_access {
            DeviceAccessConfig::Adb(adb) => Self::try_from_adb_config(ctx, adb),
            DeviceAccessConfig::Dump(_) => Self::from_env(ctx),
        }
    }

    pub fn try_from_adb_config(ctx: &dyn Context, cfg: &AdbConfig) -> crate::Result<Self> {
        let bin = cfg.get_executable(ctx)?.into_owned();
        let serial = cfg.get_serial(ctx).map(|it| it.into_owned()).ok();
        Ok(Self { bin, serial })
    }

    pub fn builder(ctx: &dyn Context) -> Builder {
        Builder::new_from_ctx(ctx)
    }
}

impl Default for ExecAdb {
    fn default() -> Self {
        Self {
            bin: "adb".into(),
            serial: None,
        }
    }
}

/// Used to build an Adb implementation.
pub struct Builder {
    bin: String,
    serial: Option<String>,
}

impl Builder {
    /// Create a new builder from the given context
    pub fn new_from_ctx(ctx: &dyn Context) -> Self {
        let bin = ctx.maybe_get_bin("adb").unwrap_or_else(|| "adb".into());
        let serial = ctx.maybe_get_env("ANDROID_SERIAL");

        Self { bin, serial }
    }

    pub fn with_bin(mut self, bin: String) -> Self {
        self.bin = bin;
        self
    }

    pub fn with_serial(mut self, serial: String) -> Self {
        self.serial = Some(serial);
        self
    }

    /// Consume the builder and return an Adb implementation
    pub fn build(self) -> ExecAdb {
        ExecAdb {
            bin: self.bin,
            serial: self.serial,
        }
    }
}

macro_rules! spawn_adb_cmd {
    ($adb:ident, $cmd:literal, $($args:expr),*) => {
        if let Some(ref serial) = $adb.serial {
            spawn_cmd(&$adb.bin, &["-s", serial, $cmd, $($args),*])
        } else {
            spawn_cmd(&$adb.bin, &[$cmd, $($args),*])
        }
    }
}

macro_rules! adb_cmd {
    ($adb:ident, $cmd:literal, $($args:expr),*) => {
        if let Some(ref serial) = $adb.serial {
            run_cmd(&$adb.bin, &["-s", serial, $cmd, $($args),*])
        } else {
            run_cmd(&$adb.bin, &[$cmd, $($args),*])
        }
    }
}

macro_rules! streamed_adb_cmd {
    ($adb:ident, $on_stdout:ident, $on_stderr:ident, $kill_child:ident, $cmd:literal, $($args:expr),*) => {
        if let Some(ref serial) = $adb.serial {
            run_cmd_streamed(&$adb.bin, &["-s", serial, $cmd, $($args),*], $on_stdout, $on_stderr, $kill_child)
        } else {
            run_cmd_streamed(&$adb.bin, &[$cmd, $($args),*], $on_stdout, $on_stderr, $kill_child)
        }
    }
}

macro_rules! split_streamed_adb_cmd {
    ($adb:ident, $split_on:ident, $on_stdout:ident, $on_stderr:ident, $cmd:literal, $($args:expr),*) => {
        if let Some(ref serial) = $adb.serial {
            run_cmd_split_streamed(&$adb.bin, &["-s", serial, $cmd, $($args),*], $split_on, $on_stdout, $on_stderr)
        } else {
            run_cmd_split_streamed(&$adb.bin, &[$cmd, $($args),*], $split_on, $on_stdout, $on_stderr)
        }
    }
}

impl ExecAdb {
    fn shell_cat_to_file(&self, device: &str, local: &str) -> io::Result<CmdOutput> {
        let shell_cat_result = adb_cmd!(self, "shell", "cat", device);

        match shell_cat_result {
            Err(e) => Err(e),
            Ok(cmd_output) => {
                fs::write(local, cmd_output.stdout)?;
                Ok(CmdOutput {
                    status: cmd_output.status,
                    stdout: Vec::new(),
                    stderr: cmd_output.stderr,
                })
            }
        }
    }
}

fn empty_result(res: io::Result<CmdOutput>) -> crate::Result<()> {
    match res {
        Ok(v) => match v.err_on_status() {
            Err(e) => Err(e),
            Ok(_) => Ok(()),
        },
        Err(e) => Err(e.into()),
    }
}

impl Adb for ExecAdb {
    /// Returns a list of all connected devices (similar to `adb devices -l`)
    fn get_connected_devices(&self) -> crate::Result<Vec<String>> {
        let output = run_cmd(&self.bin, &["devices", "-l"])?;
        let mut device_list = Vec::new();
        let out_str = output.stdout_utf8_lossy();
        let mut split = out_str.split('\n');
        // Skip the first line
        if split.next().is_none() {
            return Err(crate::Error::NoAdbDevice);
        }

        for l in split {
            if l.is_empty() || !l.contains("device") {
                continue;
            }
            if let Some(id) = l.split_ascii_whitespace().next() {
                device_list.push(id.into());
            }
        }

        if device_list.len() == 0 {
            return Err(crate::Error::NoAdbDevice);
        }

        Ok(device_list)
    }

    fn install(&self, apk: &str) -> crate::Result<()> {
        empty_result(adb_cmd!(self, "install", "-r", apk))
    }

    fn uninstall(&self, package: &str) -> crate::Result<()> {
        empty_result(adb_cmd!(self, "uninstall", package))
    }

    fn pull(&self, device: &str, local: &str) -> io::Result<CmdOutput> {
        let pull_result = adb_cmd!(self, "pull", device, local);

        match &pull_result {
            Err(_) => self.shell_cat_to_file(device, local),
            Ok(pull_cmd_output) => {
                if pull_cmd_output.status.success() {
                    pull_result
                } else {
                    self.shell_cat_to_file(device, local)
                }
            }
        }
    }

    fn spawn_pull(&self, device: &str, local: &str) -> io::Result<Child> {
        spawn_adb_cmd!(self, "pull", device, local)
    }

    fn push(&self, local: &str, device: &str) -> io::Result<CmdOutput> {
        adb_cmd!(self, "push", local, device)
    }

    fn spawn_push(&self, local: &str, device: &str) -> io::Result<Child> {
        spawn_adb_cmd!(self, "push", local, device)
    }

    fn backup(&self, dest: &PathBuf) -> io::Result<()> {
        if dest.exists() && dest.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("{} is a file, not a directory", path_must_str(dest)),
            ));
        }
        ensure_dir_exists(dest)?;
        let args = &["backup", "-all"];
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        cmd.current_dir(dest);
        if log_enabled!(Debug) {
            log_cmd(&self.bin, args);
        }

        let output: CmdOutput = cmd.output().map(|out| out.into())?;
        if let Err(e) = output.err_on_status() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("failed to backup: {}", e),
            ));
        }
        Ok(())
    }

    fn shell_streamed(
        &self,
        shell_cmd: &str,
        on_stdout: &mut OutputCallback,
        on_stderr: &mut OutputCallback,
        kill_child: Option<Receiver<()>>,
    ) -> io::Result<ExitStatus> {
        streamed_adb_cmd!(self, on_stdout, on_stderr, kill_child, "shell", shell_cmd)
    }

    fn shell(&self, shell_cmd: &str) -> io::Result<CmdOutput> {
        adb_cmd!(self, "shell", shell_cmd)
    }

    fn reverse_generic(&self, local: &str, remote: &str) -> io::Result<CmdOutput> {
        adb_cmd!(self, "reverse", local, remote)
    }

    fn forward_generic(&self, local: &str, remote: &str) -> io::Result<CmdOutput> {
        adb_cmd!(self, "forward", local, remote)
    }

    fn shell_split_streamed(
        &self,
        shell_cmd: &str,
        split_on: u8,
        on_stdout_line: &mut LineCallback,
        on_stderr_line: &mut LineCallback,
    ) -> io::Result<ExitStatus> {
        split_streamed_adb_cmd!(
            self,
            split_on,
            on_stdout_line,
            on_stderr_line,
            "shell",
            shell_cmd
        )
    }
}
