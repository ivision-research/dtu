use crossbeam::channel::RecvTimeoutError;
use crossterm::style::{ContentStyle, Stylize};
use std::time::{Duration, Instant};

use crate::printer::{color, StatusPrinter};
use crate::utils::EmptyCancelCheckThread;
use dtu::db::sql::device::diff::DiffEvent;
use dtu::tasks::ChannelEventMonitor;

pub struct PrintMonitor {
    step: String,
    count: usize,
    total: usize,
    printer: StatusPrinter,

    start: Instant,
    last_update: Instant,
}

impl PrintMonitor {
    /// Start the PrintMonitor and get a ChannelEventMonitor and JoinHandle back
    pub fn start() -> anyhow::Result<(ChannelEventMonitor<DiffEvent>, EmptyCancelCheckThread)> {
        let (mon, chan) = ChannelEventMonitor::create();
        let thread = EmptyCancelCheckThread::spawn(move |cancel_check| {
            let mut pm = Self::new();
            while !cancel_check.was_cancelled() {
                match chan.recv_timeout(Duration::from_millis(250)) {
                    Ok(it) => pm.on_event(it),
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        });
        Ok((mon, thread))
    }

    fn new() -> Self {
        let printer = StatusPrinter::new();
        printer.clear_below();
        printer.advance_line();

        Self {
            step: "".into(),
            start: Instant::now(),
            last_update: Instant::now(),
            count: 0,
            total: 0,
            printer,
        }
    }

    fn update_status_line(&mut self, do_counts: bool) {
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
                    format!("{} | {}/{}", self.step, self.count, self.total)
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
                        "{} | {}/{} | ~{:02}:{:02}:{:02} remaining",
                        self.step, self.count, self.total, h, m, s
                    )
                }
            } else {
                format!("{} | {}/{}", self.step, self.count, self.total)
            }
        } else {
            self.printer
                .update_status_line_styled(&self.step, ContentStyle::default().with(color::CYAN));
            return;
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

    fn on_started(&mut self, name: &str, count: usize) {
        self.printer.print_divider();
        self.printer.println(format!("Started {}", name));
        self.reset_count(count);
        self.update_status_line(true);
        self.step = String::from(name);
    }

    fn inc(&mut self) {
        self.count += 1;
        self.update_status_line(true);
    }

    fn on_event(&mut self, evt: DiffEvent) {
        match evt {
            DiffEvent::SystemServicesStarted { count } => {
                self.on_started("System Services", count);
            }

            DiffEvent::SystemService { .. } => {
                self.inc();
            }

            DiffEvent::SystemServicesEnded => {
                self.printer.println("System services done");
            }

            DiffEvent::ApksStarted { count } => {
                self.on_started("APKs", count);
            }
            DiffEvent::Apk { .. } => {
                self.inc();
            }
            DiffEvent::ApksEnded => {}

            DiffEvent::ReceiversStarted { count } => {
                self.on_started("Receivers", count);
            }
            DiffEvent::Receiver { .. } => {
                self.inc();
            }
            DiffEvent::ReceiversEnded => {}

            DiffEvent::ServicesStarted { count } => {
                self.on_started("Services", count);
            }
            DiffEvent::Service { .. } => {
                self.inc();
            }
            DiffEvent::ServicesEnded => {}

            DiffEvent::ProvidersStarted { count } => {
                self.on_started("Providers", count);
            }
            DiffEvent::Provider { .. } => {
                self.inc();
            }
            DiffEvent::ProvidersEnded => {}

            DiffEvent::PermissionsStarted { count } => {
                self.on_started("Permissions", count);
            }
            DiffEvent::Permission { .. } => {
                self.inc();
            }
            DiffEvent::PermissionsEnded => {}

            DiffEvent::ActivitiesStarted { count } => {
                self.on_started("Activities", count);
            }
            DiffEvent::Activity { .. } => {
                self.inc();
            }
            DiffEvent::ActivitiesEnded => {}
        }
    }
}
