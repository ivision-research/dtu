use core::time::Duration;
use std::time::Instant;

use clap::{self, Args};
use crossterm::event;
use dtu::devicefs::get_project_devicefs_helper;
use dtu::{
    db::sql::meta::get_default_metadb,
    tasks::{
        pull::{pull, Event, Options},
        ChannelEventMonitor, EventMonitor, TaskCanceller,
    },
};
use dtu::{Context, DefaultContext};
use log;
use std::thread;

use super::{applet::Applet, ui::draw};
use crate::ui::{restore_terminal, setup_terminal};

struct PrintMonitor {
    quiet: bool,
}

impl EventMonitor<Event> for PrintMonitor {
    fn on_event(&self, event: Event) {
        if self.quiet {
            return;
        }
        match event {
            Event::Pulling { device, local } => {
                log::debug!(
                    "pulling {} to {}",
                    device,
                    local.to_str().expect("sane paths")
                )
            }
            Event::FindingDirectories => {}
            _ => {}
        }
    }
}

/// Struct to hold the pull command options
#[derive(Args)]
pub struct Pull {
    /// `-n`, `--no-tui`: Flag to run without using a terminal UI. Optional
    #[arg(
        short,
        long,
        help = "Don't show the TUI",
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    no_tui: bool,

    /// `-q`, `--quiet`: Flag to disable output printing, implies `--no-tui`. Optional
    #[arg(
        short,
        long,
        help = "Don't print output, implies --no-tui",
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    quiet: bool,

    #[arg(
        long,
        help = "Force pulling and trying vdex even on later API version",
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    force_vdex: bool,

    /// The number of threads to use, 2 is the minimum
    #[arg(short = 'T', long, default_value_t = 4)]
    num_threads: usize,
}

impl Pull {
    pub fn run(&self) -> anyhow::Result<()> {
        let no_tui = self.no_tui || self.quiet;

        if no_tui {
            self.run_no_tui()
        } else {
            self.run_tui()
        }
    }

    fn get_opts(&self, ctx: &dyn Context) -> Options {
        let mut opts = Options::from_context(&ctx);
        if self.force_vdex {
            opts.try_vdex = true;
        }
        opts.worker_threads = self.num_threads;
        opts
    }

    fn run_no_tui(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let opts = self.get_opts(&ctx);

        let (cancel, check) = TaskCanceller::new();

        let pm = PrintMonitor { quiet: self.quiet };
        let meta = get_default_metadb(&ctx)?;
        let dfs = get_project_devicefs_helper(&ctx)?;

        let res = pull(&ctx, &opts, &dfs, &meta, &pm, check);

        drop(cancel);

        Ok(res?)
    }

    fn run_tui(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let opts = self.get_opts(&ctx);

        let mut res: anyhow::Result<()> = Ok(());

        let (cem, receiver) = ChannelEventMonitor::<Event>::create_with_bound(128);
        let tick_rate = Duration::from_millis(100);
        let dfs = get_project_devicefs_helper(&ctx)?;
        let meta = get_default_metadb(&ctx)?;

        thread::scope(|s| -> anyhow::Result<()> {
            let mut term = setup_terminal()?;

            let (mut canceller, check) = TaskCanceller::new();
            let mut last_tick = Instant::now();

            let mut applet = Applet::new(self, receiver);

            let mut handle = Some(s.spawn(|| pull(&ctx, &opts, &dfs, &meta, &cem, check)));

            loop {
                term.draw(|f| draw(f, &applet))?;

                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or_else(|| Duration::from_millis(0));

                if event::poll(timeout)? {
                    match event::read()? {
                        event::Event::Key(evt) => applet.handle_key_event(evt),
                        event::Event::Mouse(evt) => applet.handle_mouse_event(evt),
                        _ => {}
                    }
                }

                if last_tick.elapsed() >= tick_rate {
                    applet.on_tick();
                    last_tick = Instant::now();
                }

                if applet.should_quit_immediately() {
                    restore_terminal(&mut term)?;
                    std::process::exit(0xFF);
                } else if handle.is_some() {
                    let is_finished = handle.as_ref().unwrap().is_finished();
                    if is_finished {
                        log::trace!("pull is finished");
                        match handle.take().unwrap().join().unwrap() {
                            Err(e) => {
                                res = Err(e.into());
                                applet.is_done = true;
                                break;
                            }
                            Ok(_) => {
                                res = Ok(());
                            }
                        };

                        applet.is_done = true;

                        if applet.should_quit() {
                            break;
                        }
                    }
                } else if applet.should_quit() {
                    break;
                }

                if applet.should_quit() {
                    log::trace!("triggering canceller");
                    canceller.cancel();
                }
            }
            restore_terminal(&mut term)?;
            Ok(())
        })
        .unwrap();
        res
    }
}
