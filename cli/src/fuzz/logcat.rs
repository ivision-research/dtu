use std::fs::OpenOptions;
use std::io::{stderr, Write};
use std::path::PathBuf;
use std::thread::JoinHandle;

use clap::{self, Args};
use crossbeam::channel::{bounded, Receiver, Sender};

use crate::utils::get_adb;
use dtu::adb::Adb;
use dtu::utils::ensure_dir_exists;
use dtu::{run_cmd, Context, DefaultContext};

#[derive(Args)]
pub struct Logcat {
    /// The subdir in the logcats dir to store the output
    #[arg(long)]
    subdir: PathBuf,
}

fn start_logcat_proc(
    adb: impl Adb + 'static,
    command: String,
    output: PathBuf,
    end: Receiver<()>,
) -> anyhow::Result<JoinHandle<Result<(), String>>> {
    println!("Starting logcat process: {}", command);
    let mut output_file = OpenOptions::new()
        .truncate(true)
        .create(true)
        .write(true)
        .open(&output)?;

    Ok(std::thread::spawn(move || {
        let mut raw_stderr = Vec::new();

        adb.shell_streamed(
            &command,
            &mut |data| {
                output_file.write_all(data)?;
                Ok(())
            },
            &mut |data| {
                raw_stderr.extend(data);
                Ok(())
            },
            Some(end),
        )
        .map_err(|e| e.to_string())?;

        // res.success() will never be true because we always kill this with
        // an interrupt. Since logcat never prints to stderr, we'll consider
        // it an error if stderr has content

        if raw_stderr.len() > 0 {
            return Err(String::from_utf8_lossy(&raw_stderr).to_string());
        }

        Ok(())
    }))
}

impl Logcat {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();

        let adb = get_adb(&ctx, true)?;
        let date_string = get_date_string(&adb)?;
        let common_args = format!("-T '{}.000' -v uid -v brief -v printable", date_string);

        let mut senders = Vec::new();
        let mut handles = Vec::new();
        let res = self.launch_threads(&ctx, adb, &common_args, &mut handles, &mut senders);

        if res.is_err() {
            for s in senders {
                let _ = s.send(());
            }
        } else {
            unsafe {
                signal_hook::low_level::register(libc::SIGINT, move || {
                    for s in &senders {
                        let _ = s.send(());
                    }
                })?;
            }
        }

        for h in handles.into_iter() {
            if let Err(e) = h.join().expect("failed to join") {
                let _ = stderr().write_all(e.as_bytes());
                let _ = stderr().write(&[b'\n']);
            } else {
                log::trace!("successfully waited for logcat process");
            }
        }

        res
    }

    fn launch_threads(
        &self,
        ctx: &dyn Context,
        adb: impl Adb + Clone + 'static,
        common_args: &str,
        handles: &mut Vec<JoinHandle<Result<(), String>>>,
        senders: &mut Vec<Sender<()>>,
    ) -> anyhow::Result<()> {
        let (tx, rx) = bounded(1);
        senders.push(tx);
        let system_server_pid = self.get_system_server_pid(&adb);

        // First logcat will be all errors and verbose ssfuzz. This one is
        // mostly for development purposes, but it may help generally.
        let args = format!("logcat {} -b all '*:E' 'ssfuzz:V'", common_args);
        handles.push(start_logcat_proc(
            adb.clone(),
            args,
            self.get_available_logcat_file(&ctx, "ssfuzz_verbose_all_errors")?,
            rx,
        )?);

        let (tx, rx) = bounded(1);
        senders.push(tx);
        // Second is all errors and info ssfuzz
        let args = format!("logcat {} -b all '*:E' 'ssfuzz:I'", common_args);
        handles.push(start_logcat_proc(
            adb.clone(),
            args,
            self.get_available_logcat_file(&ctx, "ssfuzz_info_all_errors")?,
            rx,
        )?);

        let (tx, rx) = bounded(1);
        senders.push(tx);
        // Third logcat will be a very verbose logcat so we don't miss anything,
        // but we'll keep ssfuzz to I only
        let args = format!("logcat {} -b all '*:V' 'ssfuzz:I'", common_args);
        handles.push(start_logcat_proc(
            adb.clone(),
            args,
            self.get_available_logcat_file(&ctx, "ssfuzz_info_all_verbose")?,
            rx,
        )?);

        let (tx, rx) = bounded(1);
        senders.push(tx);
        // Fourth logcat is just looking for ssfuzz_default_string
        let args = format!(
            "logcat {} -b all -e 'ssfuzz_default_string' '*:V'",
            common_args
        );
        handles.push(start_logcat_proc(
            adb.clone(),
            args,
            self.get_available_logcat_file(&ctx, "all_verbose_default_string_regex")?,
            rx,
        )?);

        if let Some(pid) = system_server_pid {
            let (tx, rx) = bounded(1);
            senders.push(tx);
            let args = format!("logcat {} -b all --pid={} '*:V'", common_args, pid);
            handles.push(start_logcat_proc(
                adb.clone(),
                args,
                self.get_available_logcat_file(&ctx, "system_server_only")?,
                rx,
            )?);
        }

        Ok(())
    }

    fn get_available_logcat_file(&self, ctx: &dyn Context, name: &str) -> anyhow::Result<PathBuf> {
        let mut path = ctx.get_output_dir_child("fuzz_logcats")?.join(&self.subdir);

        ensure_dir_exists(&path)?;

        let mut i = 0;
        loop {
            path.push(format!("{}-{}", name, i));
            if !path.exists() {
                return Ok(path);
            }
            i += 1;
            path.pop();
        }
    }

    fn get_system_server_pid(&self, adb: &dyn Adb) -> Option<usize> {
        if let Ok(res) = adb.shell("pgrep system_server") {
            if res.ok() {
                if let Ok(v) = res.stdout_utf8_lossy().parse::<usize>() {
                    return Some(v);
                }
            }
        }

        if let Ok(res) = adb.shell("ps -A | grep system_server") {
            if res.ok() {
                let output = res.stdout_utf8_lossy();
                let mut raw_pid = String::new();
                let mut hit_spaces = false;
                let mut in_pid = false;
                for c in output.chars() {
                    if c.is_whitespace() {
                        if in_pid {
                            break;
                        }
                        hit_spaces = true;
                    } else if hit_spaces {
                        in_pid = true;
                        raw_pid.push(c);
                    }
                }

                if !raw_pid.is_empty() {
                    if let Ok(v) = raw_pid.parse::<usize>() {
                        return Some(v);
                    }
                }
            }
        }

        None
    }
}

fn get_date_string(adb: &dyn Adb) -> anyhow::Result<String> {
    // We're first trying to get the date from the device just in case
    // it is remote and in a different timezone.

    // The X is just a carry over from the old Python version that I was
    // too lazy to check if we still needed.
    if let Ok(res) = adb.shell("date '+%Y-%m-%dX%H:%M:%S'") {
        if res.ok() {
            return Ok(String::from_utf8(res.stdout)?.trim().replace("X", " "));
        }
    }

    // Rust doesn't have great time functionality so just reach out
    let res = run_cmd("date", &["+%Y-%m-%dX%H:%M:%S"])?.err_on_status()?;
    Ok(String::from_utf8(res.stdout)?.trim().replace("X", " "))
}
