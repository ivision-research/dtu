use std::path::Path;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::printer::{color, StatusPrinter};
use crossbeam::channel::{Receiver, RecvTimeoutError};
use crossterm::style::{ContentStyle, Stylize};
use dtu::db::graph::SetupEvent;
use dtu::utils::path_must_name;
struct PrintMonitor {
    source: String,
    count: usize,
    total: usize,
    dirs_done: usize,
    num_dirs: usize,
    printer: StatusPrinter,

    start: Instant,
    last_update: Instant,
}

impl PrintMonitor {
    fn new(step: String, num_dirs: usize) -> Self {
        Self {
            num_dirs,
            source: step,
            start: Instant::now(),
            last_update: Instant::now(),
            count: 0,
            total: 0,
            dirs_done: 0,
            printer: StatusPrinter::new(),
        }
    }

    fn update_status_line(&mut self, action: &str, do_counts: bool) {
        // No need to spam that much, just show every 10 to prove there is some
        // progress
        let now = Instant::now();
        if do_counts {
            let since_last = now.duration_since(self.last_update);
            if since_last.as_millis() < 250 {
                return;
            }
            self.last_update = now;
        }
        let status = if do_counts {
            if self.total >= 1000 {
                let elapsed = Instant::now().duration_since(self.start).as_secs();
                if elapsed == 0 {
                    format!(
                        "{} | {} | {}/{} | Total progress: {}/{}",
                        self.source, action, self.count, self.total, self.dirs_done, self.num_dirs
                    )
                } else {
                    let rate = self.count as f32 / elapsed as f32;
                    let sec_remaining =
                        ((1_f32 / rate) * (self.total - self.count) as f32).ceil() as usize;
                    let (h, m, s) = if sec_remaining < 60 {
                        (0usize, 0usize, sec_remaining as usize)
                    } else if sec_remaining < 3600 {
                        (0, sec_remaining / 60, sec_remaining % 60)
                    } else {
                        let min = sec_remaining / 60;
                        let hours = min / 60;
                        (hours, min % 60, sec_remaining % 60)
                    };
                    format!(
                        "{} | {} | {}/{} | {:.1} files/s | ~{:02}:{:02}:{:02} remaining | Total progress: {}/{}",
                        self.source, action, self.count, self.total, rate, h, m, s, self.dirs_done, self.num_dirs
                    )
                }
            } else {
                format!(
                    "{} | {} | {}/{} | Total progress: {}/{}",
                    self.source, action, self.count, self.total, self.dirs_done, self.num_dirs
                )
            }
        } else {
            format!(
                "{} | {} | Total progress: {}/{}",
                self.source, action, self.dirs_done, self.num_dirs
            )
        };
        self.printer
            .update_status_line_styled(status, ContentStyle::default().with(color::CYAN));
    }

    fn reset_count(&mut self, total: usize) {
        self.count = 0;
        self.total = total;
        self.last_update = Instant::now();
        self.start = Instant::now();
    }

    fn on_event(&mut self, evt: SetupEvent) {
        match evt {
            SetupEvent::Wiping => {
                self.printer.println(
                    "Erasing the current database contents due to force, this may take a while",
                );
            }
            SetupEvent::SmalisaStart { total_files } => {
                self.printer.print_divider();
                self.printer
                    .println(format!("Smalisa started for {}", &self.source));
                self.reset_count(total_files);
                self.update_status_line("Smalisa", true);
            }

            SetupEvent::AllImportsDone { .. } => {
                self.dirs_done += 1;
            }
            SetupEvent::AllImportsStarted { .. } => {}

            SetupEvent::SmalisaFileStarted { .. } => {}

            SetupEvent::SmalisaFileComplete { path, success } => {
                if !success {
                    log::error!("smalisa failed on {}", path);
                    self.printer
                        .println_colored(format!("path {} failed", path), color::ERROR);
                }
                self.count += 1;
                self.update_status_line("Smalisa", true);
            }
            SetupEvent::SmalisaDone => self.printer.print_divider(),
            SetupEvent::ImportStarted { path } => {
                let as_path: &Path = path.as_ref();
                self.printer
                    .print(format!("Importing {}...", path_must_name(as_path)));
                self.update_status_line("Importing CSVs", false);
            }
            SetupEvent::ImportDone { .. } => {
                self.printer.println("done");
            }
        }
    }
}

pub(crate) fn start_print_thread(
    initial_step: String,
    source_rx: Receiver<String>,
    events: Receiver<SetupEvent>,
    num_dirs: usize,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut pm = PrintMonitor::new(initial_step, num_dirs);
        loop {
            if let Ok(new_source) = source_rx.try_recv() {
                pm.source = new_source;
            }

            match events.recv_timeout(Duration::from_millis(250)) {
                Ok(it) => pm.on_event(it),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    })
}
