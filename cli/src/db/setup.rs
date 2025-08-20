use std::collections::HashMap;
use std::fs;
use std::thread::JoinHandle;

use anyhow::bail;
use clap::{self, Args};
use crossbeam::channel::Receiver;
use crossterm::style::{ContentStyle, Stylize};
use dtu::db::graph::get_default_graphdb;

use crate::printer::{color, StatusPrinter};
use dtu::db::sql::device::{
    get_project_dbsetup_helper, ApkIdentifier, DiffOptions, DiffTask, SetupEvent, SetupOptions,
    EMULATOR_DIFF_SOURCE,
};
use dtu::db::sql::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::tasks::{ChannelEventMonitor, TaskCanceller};
use dtu::utils::{path_must_str, DevicePath};
use dtu::{Context, DefaultContext};

use super::monitor::PrintMonitor;
use super::{get_aosp_database, get_aosp_database_path, get_path_for_diff_source};

#[derive(Args)]
pub struct Setup {
    /// Force the database to be reset if it already exists
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    force: bool,

    /// Don't show progress output
    #[arg(short, long)]
    quiet: bool,

    /// Don't perform a diff (must be set for emulator databases)
    #[arg(
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    no_diff: bool,

    /// API level for diffing, otherwise taken from device
    #[arg(short = 'A', long)]
    api_level: Option<u32>,
}

pub(super) struct ErrorReporter {
    failed_apks: Vec<DevicePath>,
}

impl Default for ErrorReporter {
    fn default() -> Self {
        Self {
            failed_apks: Vec::new(),
        }
    }
}

impl ErrorReporter {
    fn failed(&self) -> bool {
        self.failed_apks.len() > 0
    }

    pub(super) fn as_err(&self) -> anyhow::Result<()> {
        if !self.failed() {
            return Ok(());
        }
        let mut err_string = String::new();
        if self.failed_apks.len() > 0 {
            err_string.push_str(&format!(
                "failed to import {} apks:\n",
                self.failed_apks.len()
            ));
            for apk in &self.failed_apks {
                err_string.push_str(&format!("   - {}\n", apk));
            }
        }

        Err(anyhow::Error::msg(err_string))
    }
}

fn update_service_status(
    printer: &StatusPrinter,
    done: usize,
    count: usize,
    svc_name: Option<&str>,
) {
    let msg = if let Some(name) = svc_name {
        format!("System Services | {}/{} | {}", done, count, name)
    } else {
        format!("System Services | {}/{}", done, count)
    };
    printer.update_status_line_styled(msg, ContentStyle::default().with(color::CYAN));
}

fn update_apk_status(
    printer: &StatusPrinter,
    apks: &HashMap<usize, DevicePath>,
    done: usize,
    count: usize,
    id: ApkIdentifier,
) {
    let apk = apks.get(&id.apk_id);
    let apk_str = match apk {
        Some(v) => v.as_device_str(),
        None => "?",
    };
    let status = format!("Apks | {}/{} | {}", done, count, apk_str);
    printer.update_status_line_styled(status, ContentStyle::default().with(color::CYAN));
}

pub(super) fn start_monitor_thread(
    silent: bool,
    chan: Receiver<SetupEvent>,
) -> JoinHandle<ErrorReporter> {
    std::thread::spawn(move || {
        let mut errs = ErrorReporter::default();

        let mut printer = StatusPrinter::new();
        printer.set_silent(silent);

        let mut services: HashMap<usize, String> = HashMap::new();
        let mut apks: HashMap<usize, DevicePath> = HashMap::new();

        let mut done: usize = 0;
        let mut count: usize = 0;

        loop {
            let evt = match chan.recv() {
                Ok(v) => v,
                Err(_) => break,
            };
            match evt {
                SetupEvent::DiscoveredServices { entries } => {
                    done = 0;
                    count = entries.len();
                }
                SetupEvent::StartAddingSystemService {
                    service,
                    service_id,
                    ..
                } => {
                    update_service_status(&printer, done, count, Some(service.as_str()));
                    services.insert(service_id, service);
                }
                SetupEvent::FoundSystemServiceImpl {
                    service_id,
                    implementation,
                    source,
                } => {
                    let svc = services.get(&service_id);
                    if let Some(s) = svc {
                        printer.print_colored(implementation, color::GREEN);
                        printer.print(" | ");
                        printer.print_colored(s, color::CYAN);
                        printer.print(" | ");
                        printer.println_colored(source, color::PURPLE);
                    }
                }

                SetupEvent::FoundSystemServiceMethod { .. } => {}
                SetupEvent::DoneAddingSystemService { service_id } => {
                    done += 1;
                    services.remove(&service_id);
                    update_service_status(&printer, done, count, None);
                }

                SetupEvent::StartedApksForDir { count: c, .. } => {
                    done = 0;
                    count = c;
                }
                SetupEvent::DoneApksForDir { .. } => {}
                SetupEvent::StartAddingApk { path, identifier } => {
                    printer.println(path.as_device_str());
                    apks.insert(identifier.apk_id, path);
                    update_apk_status(&printer, &apks, done, count, identifier);
                }
                SetupEvent::DoneAddingApk {
                    success,
                    identifier,
                } => {
                    if let Some(path) = apks.remove(&identifier.apk_id) {
                        if !success {
                            errs.failed_apks.push(path);
                        }
                    }
                    done += 1;
                }
                SetupEvent::AddingApkPermission { .. } => {}
                SetupEvent::AddingApkProvider { .. } => {}
                SetupEvent::AddingApkActivity { .. } => {}
                SetupEvent::AddingApkService { .. } => {}
                SetupEvent::AddingApkReceiver { .. } => {}
            }
        }

        printer.print("\n");

        errs
    })
}

impl Setup {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let mdb = MetaSqliteDatabase::new(&ctx)?;

        mdb.ensure_prereq(Prereq::GraphDatabasePartialSetup)?;

        let helper = get_project_dbsetup_helper(&ctx)?;

        let db = DeviceSqliteDatabase::new(&ctx)?;
        let graph = get_default_graphdb(&ctx)?;

        let (mon, chan) = ChannelEventMonitor::create();
        let thread_handle = start_monitor_thread(self.quiet, chan);

        let (_cancel, check) = TaskCanceller::new();

        let opts = SetupOptions::default().set_force(self.force);

        db.setup(&ctx, opts, &helper, Some(&mon), &graph, &mdb, check)?;

        drop(mon);

        let err_reporter = thread_handle.join().expect("failed to join thread");

        if self.no_diff {
            return err_reporter.as_err();
        }

        let source = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
        let (_cancel, check) = TaskCanceller::new();
        let (mon, _join) = PrintMonitor::start()?;
        let opts = DiffOptions::new(source);

        let api_level = self.api_level.unwrap_or_else(|| ctx.get_target_api_level());
        let aosp_db = get_aosp_database(&ctx, api_level)?;
        let task = DiffTask::new(opts, &db, &aosp_db, check, &mon);

        let res = task.run();
        drop(mon);
        res?;

        mdb.update_prereq(Prereq::EmulatorDiff, true)?;

        let src_path = get_aosp_database_path(&ctx, api_level)?;
        let path = get_path_for_diff_source(&ctx, EMULATOR_DIFF_SOURCE)?;
        if let Err(e) = fs::copy(&src_path, &path) {
            bail!(
                "failed to copy {} to {}: {}",
                path_must_str(&src_path),
                path_must_str(&path),
                e
            );
        }

        err_reporter.as_err()
    }
}
