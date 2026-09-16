#![allow(dead_code)]
use anyhow::bail;
use std::path::PathBuf;

use clap::Parser;

use dtu::decompile::decompile_file;
use dtu::devicefs::get_project_devicefs_helper;
use dtu::utils::fs::path_must_str;
use dtu::DefaultContext;

struct StderrLogger {
    level: log::Level,
}

impl StderrLogger {
    fn new(level: log::Level) -> Self {
        Self { level }
    }
}

impl log::Log for StderrLogger {
    fn log(&self, record: &log::Record) {
        if record.level() > self.level {
            return;
        }

        if let Some(path) = record.module_path() {
            eprintln!("{}:{}: {}", path, record.level(), record.args());
        } else {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }
}

#[derive(Parser)]
struct Cli {
    #[arg(short, long, help = "Debug output")]
    debug: bool,

    #[arg(short, long, help = "Trace output")]
    trace: bool,

    #[arg(short, long, help = "Input file")]
    file: PathBuf,

    #[arg(short, long, help = "Output dir")]
    out: PathBuf,

    #[arg(short, long, help = "Android API level")]
    api: Option<u32>,
}
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let lvl = if cli.trace {
        log::Level::Trace
    } else if cli.debug {
        log::Level::Debug
    } else {
        log::Level::Info
    };
    let logger = Box::new(StderrLogger::new(lvl));
    log::set_boxed_logger(logger).map(|()| log::set_max_level(lvl.to_level_filter()))?;

    let mut ctx = DefaultContext::new();
    if let Some(api) = cli.api {
        ctx.set_target_api_level(api);
    }
    let dfs = get_project_devicefs_helper(&ctx)?;
    let success = decompile_file(&ctx, &dfs, path_must_str(&cli.file), &cli.out)?;
    if !success {
        bail!("decompilation failed, but there was no error");
    }
    Ok(())
}
