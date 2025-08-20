use std::{borrow::Cow, env::current_dir, path::PathBuf};

use clap::{Args, Parser, Subcommand};
use dtu::{
    filestore::{get_filestore, FileStore},
    utils::{path_must_name, path_must_str},
    Context, DefaultContext,
};

type Fs = Box<dyn FileStore>;

#[derive(Args)]
struct Get {
    /// Path to retrieve
    remote: String,
    /// Optional path to write to
    local: Option<PathBuf>,
}

#[derive(Args)]
struct Put {
    /// Path of item to put
    local: PathBuf,
    /// Path in the file store
    remote: String,
}

#[derive(Args)]
struct Rm {
    /// Path to remove
    path: String,
}

#[derive(Args)]
struct List {
    /// Prefix or directory to list
    prefix: Option<String>,
}

#[derive(Parser)]
#[command(name = "dtu-fs")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List items in the file store
    List(List),
    /// Remove an item from the file store
    Rm(Rm),
    /// Put an item into the file store
    Put(Put),
    /// Get an item from the file store
    Get(Get),
}

fn get(ctx: &dyn Context, fs: Fs, args: Get) -> anyhow::Result<()> {
    let to = match &args.local {
        Some(v) => Cow::Borrowed(v),
        None => {
            let name = path_must_name(args.remote.as_ref());
            Cow::Owned(PathBuf::from(current_dir()?).join(name))
        }
    };
    Ok(fs.get_file(ctx, &args.remote, path_must_str(&to))?)
}

fn put(ctx: &dyn Context, fs: Fs, args: Put) -> anyhow::Result<()> {
    Ok(fs.put_file(ctx, path_must_str(&args.local), &args.remote)?)
}

fn list(ctx: &dyn Context, fs: Fs, args: List) -> anyhow::Result<()> {
    let files = fs.list_files(ctx, args.prefix.as_ref().map(String::as_str))?;
    for f in files {
        println!("{f}");
    }
    Ok(())
}

fn rm(ctx: &dyn Context, fs: Fs, args: Rm) -> anyhow::Result<()> {
    Ok(fs.remove_file(ctx, &args.path)?)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let ctx = DefaultContext::new();
    let fs = get_filestore(&ctx)?;

    match cli.command {
        Commands::Rm(args) => rm(&ctx, fs, args),
        Commands::List(args) => list(&ctx, fs, args),
        Commands::Put(args) => put(&ctx, fs, args),
        Commands::Get(args) => get(&ctx, fs, args),
    }?;

    Ok(())
}
