use clap::{self, Args};
use std::borrow::Cow;
use std::fs::File;
use std::io::{stderr, stdout, Read, Write};
use std::path::PathBuf;
use std::process::exit;

use crate::utils::get_app_server;
use dtu::app::server::AppServer;
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[derive(Args)]
pub struct Sh {
    /// A file to read the command from
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Optionally set a sh binary on the device
    ///
    /// If this isn't set, the application will choose the default shell
    #[arg(long)]
    sh: Option<String>,

    /// A command to be run as sh -c '...'
    ///
    /// This allows for things like: 'echo test; sleep 1; echo test2'
    #[arg(short = 'c', long = "cmd")]
    sh_cmd: Option<String>,

    /// Shell command to be run (otherwise read from stdin)
    #[arg()]
    cmd: Vec<String>,
}

impl Sh {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::AppSetup)?;

        let cmd = self.get_command()?;
        let shell = self.sh.as_ref().map(|it| it.as_str());
        let mut srv = get_app_server(&ctx)?;
        let res = srv.sh_with_shell(cmd.as_ref(), shell)?;

        stdout().write_all(res.stdout.as_slice())?;
        stderr().write_all(res.stderr.as_slice())?;
        exit(res.exit)
    }

    fn get_command(&self) -> anyhow::Result<Cow<'_, str>> {
        if let Some(cmd) = &self.sh_cmd {
            return Ok(Cow::Borrowed(cmd));
        }

        let mut command = String::new();
        let len = self.cmd.len();
        if len > 0 {
            for (i, c) in self.cmd.iter().enumerate() {
                command.push_str(c);
                if i < len - 1 {
                    command.push(' ');
                }
            }
            return Ok(Cow::Owned(command));
        }

        if let Some(path) = &self.file {
            let mut f = File::open(path)?;
            f.read_to_string(&mut command)?;
        } else {
            let mut stdin = std::io::stdin();
            stdin.read_to_string(&mut command)?;
        }

        Ok(Cow::Owned(command))
    }
}
