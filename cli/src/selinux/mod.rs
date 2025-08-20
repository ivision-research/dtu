use clap::{self, Args, Subcommand};
use dtu::db::sql::MetaSqliteDatabase;
use dtu::devicefs::get_project_devicefs_helper;
use dtu::tasks::selinux::{pull, Options};
use dtu::tasks::NoopMonitor;
use dtu::DefaultContext;

#[derive(Args)]
pub struct Selinux {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pull the Selinux policy
    #[command()]
    Pull(Pull),
}

impl Selinux {
    pub fn run(&self) -> anyhow::Result<()> {
        match &self.command {
            Command::Pull(c) => c.run(),
        }
    }
}

#[derive(Args)]
struct Pull {
    /// Run even if the policy has already been built
    #[arg(short, long)]
    force: bool,
}

impl Pull {
    fn run(&self) -> anyhow::Result<()> {
        let mon = NoopMonitor::new();
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        let dfs = get_project_devicefs_helper(&ctx)?;
        let opts = Options { force: false };
        pull(&ctx, &meta, &mon, &dfs, &opts)?;
        Ok(())
    }
}
