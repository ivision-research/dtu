use std::borrow::Cow;
use std::path::PathBuf;

use anyhow::Context as AnyhowContext;
use clap::{Parser, Subcommand};
use flexi_logger::{FileSpec, LevelFilter, LogSpecification, Logger, LoggerHandle, WriteMode};

use dtu::{Context, DefaultContext};

mod gen_envrc;
mod parsers;
mod progress;
use gen_envrc::GenEnvrc;

mod circular;
mod printer;

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
    /// `-e`, `--log-stderr`: Flag value, when enabled will cause logs to be output to `stderr`
    /// instead of a log file. Disabled by default (logs go to a file by default)
    #[arg(short = 'e', long, help = "Log to stderr instead of a file", action = clap::ArgAction::SetTrue, default_value_t = false)]
    log_stderr: bool,

    /// `-f`, `--log-file`: Path to desired log output file location. Optional, defaults to
    /// `$DTU_PROJECT_HOME/dtu_out/log`
    #[arg(short = 'f', long, help = "Send log output to the given file")]
    log_file: Option<PathBuf>,

    /// `-s`, `--log-spec`: Debug options for [flexi_logger](https://docs.rs/flexi_logger/0.24.2/flexi_logger/struct.LogSpecification.html)
    #[arg(short = 's', long, help = "Log spec for flexi_logger")]
    log_spec: Option<String>,

    /// `-l`, `--log-level`: Set the desired log verbosity. Defaults to 0, all values are listed
    /// below:
    ///
    /// | Value | Log Level |
    /// | ----- | --------- |
    /// | **0** | **Warn** |
    /// | 1 | Info |
    /// | 2 | Debug |
    /// | 3 | Trace |
    #[arg(
        short = 'l',
        long,
        help = "Set the log level, 0 = warn, 1 = info, etc",
        long_help = None,
        default_value_t = 0
    )]
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
    ///{n}
    /// null          - Writes a null{n}
    /// bind          - Writes one of the applicaton's `LoggingBinder`s{n}
    ///{n}
    /// list 1 ... N end  - Writes a list/array. The elements of the array{n}
    ///                     are specified in the same language{n}
    ///{n}
    /// map : k v ... k v end  - Writes a map. The keys and values can both{n}
    ///                          be arbitrary values{n}
    ///{n}
    /// bund : key v .. end  -  Writes a Bundle. The keys must be strings, you{n}
    ///                         do not need to specify `str` in front of them{n}
    ///                         since they're guaranteed to be strings.{n}
    #[command()]
    Call(Call),

    /// Selinux related commands
    #[command()]
    Selinux(Selinux),
}

impl Cli {
    fn configure_loggers(&self, ctx: &DefaultContext) -> anyhow::Result<LoggerHandle> {
        let log_spec = match &self.log_spec {
            Some(s) => {
                LogSpecification::parse(s).with_context(|| format!("parsing log spec {}", s))?
            }
            None => {
                if self.log_level > 0 {
                    let lvl = if self.log_level == 1 {
                        LevelFilter::Info
                    } else if self.log_level == 2 {
                        LevelFilter::Debug
                    } else {
                        LevelFilter::Trace
                    };
                    LogSpecification::builder().module("dtu", lvl).build()
                } else {
                    LogSpecification::env().with_context(|| "getting log spec from env")?
                }
            }
        };

        let mut logger = Logger::with(log_spec);

        if !self.log_stderr {
            let path = match &self.log_file {
                Some(v) => {
                    if v.is_absolute() {
                        Some(Cow::Borrowed(v))
                    } else {
                        let full_path = std::env::current_dir()?.join(v);
                        Some(Cow::Owned(full_path))
                    }
                }
                None => ctx.get_output_dir_child("log").map(Cow::Owned).ok(),
            };

            if let Some(p) = &path {
                logger = logger
                    .log_to_file(
                        FileSpec::try_from(p.as_ref()).with_context(|| "creating filespec")?,
                    )
                    .append()
                    .write_mode(WriteMode::BufferAndFlush);
            }
        }

        Ok(logger.start().with_context(|| "starting logger")?)
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Commands::Version = &cli.command {
        println!("{}", VERSION_STRING);
        return Ok(());
    }

    let ctx = DefaultContext::default();

    let log_handle = cli.configure_loggers(&ctx)?;

    let res = match cli.command {
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

    log_handle.flush();
    res
}
