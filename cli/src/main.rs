use std::fs::OpenOptions;
use std::path::PathBuf;

use anyhow::Context as AnyhowContext;
use clap::{Parser, Subcommand};

use dtu::{Context, DefaultContext};

mod gen_envrc;
mod parsers;
mod progress;
use gen_envrc::GenEnvrc;

mod circular;
mod logging;
mod printer;
use logging::{FileLogger, StderrLogger};

mod pull;
use pull::Pull;

mod call;
use call::Call;

mod diff;
use diff::Diff;

mod db;
use db::DB;

mod graph;
mod utils;
use graph::Graph;

mod meta;
use meta::Meta;

mod find;
use find::Find;

mod open_file;
use open_file::OpenSmaliFile;

mod list;
use list::List;

mod app;
use app::App;

mod broadcast;
use broadcast::Broadcast;

mod start_activity;
use start_activity::StartActivity;

mod start_service;
use start_service::StartService;

mod provider;
use provider::Provider;

pub mod ui;

mod fuzz;
use fuzz::Fuzz;

mod sh;
use sh::Sh;

mod shell_cmd;
use shell_cmd::ShellCmd;

mod check;
use check::RunCheck;

mod selinux;
use selinux::Selinux;

mod scripting;
use scripting::Scripting;

mod taint;
use taint::Taint;

#[cfg(test)]
mod testing;

const SIMPLE_VERSION_STRING: &'static str =
    include!(concat!(env!("OUT_DIR"), "/simple_version_string"));
const VERSION_STRING: &'static str = include!(concat!(env!("OUT_DIR"), "/version_string"));

#[derive(Parser)]
#[command(name = "dtu")]
#[command(version(SIMPLE_VERSION_STRING))]
#[command(long_version(VERSION_STRING))]
struct Cli {
    /// Log to stderr instead of a file
    #[arg(short = 'e', long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    log_stderr: bool,

    /// Path to desired log output file location. Defaults to `$DTU_PROJECT_HOME/dtu_out/log`
    #[arg(short = 'f', long)]
    log_file: Option<PathBuf>,

    /// Set the desired log verbosity. Defaults to 0 (warn) up to 3 (trace)
    #[arg(short = 'l', long, default_value_t = 0)]
    log_level: u8,

    /// The command being called. See [Commands] for the implemented options
    #[command(subcommand)]
    command: Commands,
}

/// The currently implemented commands
#[derive(Subcommand)]
enum Commands {
    /// Display the full version string and exit
    #[command()]
    Version,

    /// Write an `.envrc` file to setup an environment for all other commands.
    ///
    /// This should generally be run as the first step in any project, as
    /// the other commands all expect some environmental variables to be set.
    #[command()]
    GenEnvrc(GenEnvrc),
    /// Pull and decompile the framework files and system level APKs.
    ///
    /// This operation takes a fairly long time and is resource intensive, but
    /// it is essential to every other command.
    #[command()]
    Pull(Pull),

    /// Operations on the device database
    #[command()]
    DB(DB),

    /// Graph database operations
    #[command()]
    Graph(Graph),

    /// Operations on the meta database
    #[command()]
    Meta(Meta),

    /// Open a smali file in the $EDITOR text editor
    #[command(alias = "of")]
    OpenSmaliFile(OpenSmaliFile),

    /// Generic listing commands
    #[command()]
    List(List),

    /// Lookup info from the databases
    #[command()]
    Find(Find),

    /// View differences with AOSP
    #[command()]
    Diff(Diff),

    /// Interact with the test application
    #[command()]
    App(App),

    /// Use the test application to send a broadcast
    ///
    /// This is better than using `adb shell am broadcast ...` because the
    /// broadcast is sent from the test application and not as the shell user.
    #[command()]
    Broadcast(Broadcast),

    /// Use the test application to start an activity
    ///
    /// Similar to `broadcast`
    #[command()]
    StartActivity(StartActivity),

    /// Use the test application to start a service
    ///
    /// Similar to `broadcast`
    #[command()]
    StartService(StartService),

    /// Use the test application to interact with a provider
    #[command()]
    Provider(Provider),

    /// Operations related to fuzzing with ssfuzz/fast
    #[command()]
    Fuzz(Fuzz),

    /// Run a shell command as the test application
    ///
    /// Note that `sh` depends on the test application being up and running
    #[command()]
    Sh(Sh),

    /// Run a service's shell command handler
    ///
    /// Note that `shell-cmd` depends on the test application being up and running
    #[command()]
    ShellCmd(ShellCmd),

    /// Check to see if you are able to use `dtu`
    #[command()]
    RunCheck(RunCheck),

    /// Call a method on an application or system service
    ///
    /// These commands have some limited support for sending arbitrary
    /// parcels. The syntax is as follows:
    ///
    /// i64 <i64>     - Write a long{n}
    /// i32 <i32>     - Write an int{n}
    /// i16 <i16>     - Writes a short{n}
    /// u8 <u8>       - Writes a byte{n}
    /// z true|false  - Writes a boolean{n}
    /// f64 <f64>     - Writes a double{n}
    /// f32 <f32>     - Writes a float{n}
    /// str <str>     - Writes a string{n}
    /// wfd <str>     - Writes a file descriptor opened r/w{n}
    /// rfd <str>     - Writes a file descriptor opened read only{n}
    /// bar <hexstr>  - Writes a raw byte array input as hex{n}
    ///{n}
    /// null          - Writes a null{n}
    /// bind          - Writes one of the applicaton's `LoggingBinder`s{n}
    ///{n}
    /// list 1 ... N end  - Writes a list/array. The elements of the array{n}
    ///                     are specified in the same language{n}
    ///{n}
    /// map k v ... k v end  - Writes a map. The keys and values can both{n}
    ///                        be arbitrary values{n}
    ///{n}
    /// bund k v .. end  -  Writes a Bundle. The keys must be strings, you{n}
    ///                     do not need to specify `str` in front of them{n}
    ///                     since they're guaranteed to be strings.{n}
    ///{n}
    /// msg <i32> <i32> <i32> [bund k v ...] end{n}
    ///{n}
    /// Writes a Message type, the `what`, `arg1`, and `arg2` are requred and{n}
    /// an optional `bund` may be supplied followed by a required `end`.{n}
    ///{n}
    ///{n}
    /// Note that this is a very low level interface. If a type may be nullable{n}
    /// you will need to include the flag yourself. For example, the `msg` type{n}
    /// will need to be proceeded by `i32 1` to tag it as non null. This may{n}
    /// be changed in the future
    #[command()]
    Call(Call),

    /// Selinux related commands
    #[command()]
    Selinux(Selinux),

    /// Taint analysis related commands
    #[command()]
    Taint(Taint),

    #[command(name = "_scripting")]
    #[command(alias = "_s")]
    #[command(hide = true)]
    Scripting(Scripting),
}

impl Cli {
    fn configure_loggers(&mut self, ctx: &DefaultContext) -> anyhow::Result<()> {
        let mut level = match self.log_level {
            0 => log::Level::Warn,
            1 => log::Level::Info,
            2 => log::Level::Debug,
            _ => log::Level::Trace,
        };

        // respect RUST_LOG=$LEVEL syntax but no others
        if let Some(env) = ctx.maybe_get_env("RUST_LOG") {
            if let Ok(parsed) = env.parse::<log::Level>() {
                level = parsed;
            }
        }

        if self.log_stderr {
            return self.stderr_logger(level);
        }

        let path = match self.log_file.take() {
            Some(v) => v,
            None => ctx.get_output_dir_child("log")?,
        };

        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("opening log file: {}", path.display()))?;

        let logger = Box::new(FileLogger::new(file, level));
        log::set_boxed_logger(logger).map(|()| log::set_max_level(level.to_level_filter()))?;

        Ok(())
    }

    fn stderr_logger(&self, level: log::Level) -> anyhow::Result<()> {
        let logger = Box::new(StderrLogger::new(level));
        log::set_boxed_logger(logger).map(|()| log::set_max_level(level.to_level_filter()))?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();

    if let Commands::Version = &cli.command {
        println!("{}", VERSION_STRING);
        return Ok(());
    }

    let ctx = DefaultContext::default();

    cli.configure_loggers(&ctx)?;

    let res = match cli.command {
        Commands::Taint(c) => c.run(),
        Commands::Scripting(c) => c.run(),
        Commands::Pull(c) => c.run(),
        Commands::GenEnvrc(c) => c.run(),
        Commands::DB(c) => c.run(),
        Commands::Graph(c) => c.run(),
        Commands::Meta(c) => c.run(),
        Commands::OpenSmaliFile(c) => c.run(),
        Commands::List(c) => c.run(),
        Commands::Find(c) => c.run(),
        Commands::Diff(c) => c.run(),
        Commands::App(c) => c.run(),
        Commands::Broadcast(c) => c.run(),
        Commands::StartActivity(c) => c.run(),
        Commands::StartService(c) => c.run(),
        Commands::Provider(c) => c.run(),
        Commands::Fuzz(c) => c.run(),
        Commands::Sh(c) => c.run(),
        Commands::ShellCmd(c) => c.run(),
        Commands::Call(c) => c.run(),
        Commands::RunCheck(c) => c.run(),
        Commands::Selinux(c) => c.run(),

        Commands::Version => panic!("unreachable"),
    };

    res
}
