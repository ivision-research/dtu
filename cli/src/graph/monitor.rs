use std::path::Path;
use std::time::{Duration, Instant};

use crate::printer::{color, StatusPrinter};
use crate::utils::EmptyCancelCheckThread;
use crossbeam::channel::{Receiver, RecvTimeoutError};
use crossterm::style::{ContentStyle, Stylize};
use dtu::db::graph::SetupEvent;
use dtu::tasks::smalisa;
use dtu::utils::path_must_name;

struct SmalisaPrintMonitor {
    source: String,
    count: u32,
    total: u32,
    total_count: u32,
    sources_done: usize,
    num_sources: usize,
    printer: StatusPrinter,

    start: Instant,
    last_update: Instant,
}

impl SmalisaPrintMonitor {
    fn new(step: String, num_sources: usize) -> Self {
        Self {
            num_sources,
            source: step,
            start: Instant::now(),
            last_update: Instant::now(),
            count: 0,
            total: 0,
            total_count: 0,
            sources_done: 0,
            printer: StatusPrinter::new(),
        }
    }

    // Return the rate as files per second
    #[inline]
    fn get_rate(&self, now: Instant) -> f64 {
        let elapsed = now.duration_since(self.start).as_secs() as u32;
        f64::from(self.total_count + self.count) / f64::from(elapsed)
    }

    fn get_remaining_hms(&self, rate: f64) -> (usize, usize, usize) {
        let sec_remaining = ((1f64 / rate) * f64::from(self.total - self.count)).ceil() as usize;

        if sec_remaining < 60 {
            (0usize, 0usize, sec_remaining as usize)
        } else if sec_remaining < 3600 {
            (0, sec_remaining / 60, sec_remaining % 60)
        } else {
            let min = sec_remaining / 60;
            let hours = min / 60;
            (hours, min % 60, sec_remaining % 60)
        }
    }

    fn update_status_line(&mut self) {
        // No need to spam that much, just show every 250ms to prove there is some
        // progress
        let now = Instant::now();
        let since_last = now.duration_since(self.last_update);
        if since_last.as_millis() < 250 {
            return;
        }
        self.last_update = now;
        let elapsed = now.duration_since(self.start).as_secs();
        let status = if elapsed == 0 {
            format!(
                "{} | {}/{} | Total progress: {}/{}",
                self.source, self.count, self.total, self.sources_done, self.num_sources
            )
        } else {
            let rate = self.get_rate(now);
            let (h, m, s) = self.get_remaining_hms(rate);
            format!(
                "{} | {}/{} | {:.1} files/s | ~{:02}:{:02}:{:02} remaining | Total progress: {}/{}",
                self.source,
                self.count,
                self.total,
                rate,
                h,
                m,
                s,
                self.sources_done,
                self.num_sources
            )
        };
        self.printer
            .update_status_line_styled(status, ContentStyle::default().with(color::CYAN));
    }

    fn reset_count(&mut self, total: u32) {
        self.total_count += self.count;
        self.count = 0;
        self.total = total;
        self.last_update = Instant::now();
    }

    fn on_event(&mut self, evt: smalisa::Event) {
        match evt {
            smalisa::Event::Start { total_files } => {
                self.printer.print(format!("Smalisa {}...", &self.source));
                self.reset_count(total_files.try_into().unwrap_or(total_files as u32));
                self.update_status_line();
            }

            smalisa::Event::FileStarted { .. } => {}

            smalisa::Event::FileComplete { path, success } => {
                if !success {
                    log::error!("smalisa failed on {}", path);
                    self.printer
                        .println_colored(format!("path {} failed", path), color::ERROR);
                }
                self.count += 1;
                self.update_status_line();
            }
            smalisa::Event::Done { success } => {
                let (color_, status) = if success {
                    (color::OK, " OK")
                } else {
                    log::error!("smalisa failed for {}", self.source);
                    (color::ERROR, " ERR")
                };

                self.printer.println_colored(status, color_);
                self.sources_done += 1;
                self.update_status_line();
            }
        }
    }
}

struct ImportPrintMonitor {
    source: String,
    sources_done: usize,
    num_sources: usize,
    printer: StatusPrinter,
}

impl ImportPrintMonitor {
    fn new(step: String, num_sources: usize) -> Self {
        Self {
            num_sources,
            source: step,
            sources_done: 0,
            printer: StatusPrinter::new(),
        }
    }

    fn show_status(&self) {
        let status = format!(
            "{} | Total progress: {}/{}",
            self.source, self.sources_done, self.num_sources
        );
        self.printer
            .update_status_line_styled(status, ContentStyle::default().with(color::CYAN));
    }

    fn on_event(&mut self, evt: SetupEvent) {
        match evt {
            SetupEvent::Wiping => {
                self.printer.println(
                    "Erasing the current database contents due to force, this may take a while",
                );
            }
            SetupEvent::SourceDone { .. } => {
                self.sources_done += 1;
                self.show_status();
            }
            SetupEvent::SourceStarted { source } => {
                self.source = source;
                self.show_status();
            }

            SetupEvent::ImportStarted { path } => {
                let as_path: &Path = path.as_ref();
                self.printer
                    .print(format!("Importing {}...", path_must_name(as_path)));
                self.show_status();
            }
            SetupEvent::ImportDone { .. } => {
                self.printer.println("done");
            }
        }
    }
}

pub(crate) fn start_smalisa_print_thread(
    initial_step: String,
    source_rx: Receiver<String>,
    events: Receiver<smalisa::Event>,
    num_sources: usize,
) -> EmptyCancelCheckThread {
    EmptyCancelCheckThread::spawn(move |cancel_check| {
        let mut pm = SmalisaPrintMonitor::new(initial_step, num_sources);
        while !cancel_check.was_cancelled() {
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

pub(crate) fn start_import_print_thread(
    initial_step: String,
    events: Receiver<SetupEvent>,
    num_sources: usize,
) -> EmptyCancelCheckThread {
    EmptyCancelCheckThread::spawn(move |cancel_check| {
        let mut pm = ImportPrintMonitor::new(initial_step, num_sources);
        while !cancel_check.was_cancelled() {
            match events.recv_timeout(Duration::from_millis(250)) {
                Ok(it) => pm.on_event(it),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    })
}
