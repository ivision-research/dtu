use clap::{self, Args, Subcommand};
use std::borrow::Cow;
use std::io::{stderr, stdout, Read, Write};
use std::process::exit;

use crate::parsers::SystemServiceValueParser;
use crate::utils::get_app_server;
use dtu::app::server::AppServer;
use dtu::db::device::models;
use dtu::db::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::{Context, DefaultContext};

#[derive(Args)]
pub struct ShellCmd {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Perform a ShellCommand call on a system
    SystemService(SystemService),
}

#[derive(Args)]
struct SystemService {
    /// The system service name
    #[arg(short, long, value_parser = SystemServiceValueParser)]
    service: models::SystemService,

    /// The shell command string, set to `-` to read from stdin
    #[arg(short, long)]
    command: Option<String>,

    /// Timeout (ms) to wait for the command to complete
    #[arg(short, long, default_value_t = 2500)]
    timeout: u32,
}

impl ShellCmd {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::AppSetup)?;
        match &self.command {
            Command::SystemService(c) => c.run(&ctx),
        }
    }
}

impl SystemService {
    pub fn run(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        let mut srv = get_app_server(ctx)?;

        let cmd = self.get_command()?;

        let cmd_str = cmd.as_ref().map(|it| it.as_ref());

        let res = srv.system_service_shell_cmd(&self.service.name, cmd_str, self.timeout)?;

        stdout().write_all(res.stdout.as_slice())?;
        stderr().write_all(res.stderr.as_slice())?;
        exit(res.exit)
    }

    fn get_command(&self) -> anyhow::Result<Option<Cow<'_, str>>> {
        let cmd = match &self.command {
            None => return Ok(None),
            Some(v) => v,
        };
        if cmd != "-" {
            return Ok(Some(Cow::Borrowed(cmd)));
        }
        let mut command = String::new();
        let mut stdin = std::io::stdin();
        stdin.read_to_string(&mut command)?;

        Ok(Some(Cow::Owned(command)))
    }
}
