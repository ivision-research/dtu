#![allow(dead_code)]
use anyhow::bail;
use std::path::PathBuf;

use clap::Parser;
use flexi_logger::{LevelFilter, LogSpecification, Logger};

use dtu::decompile::decompile_file;
use dtu::devicefs::get_project_devicefs_helper;
use dtu::utils::fs::path_must_str;
use dtu::DefaultContext;

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
        LevelFilter::Trace
    } else if cli.debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    Logger::with(LogSpecification::builder().module("dtu", lvl).build()).start()?;
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
