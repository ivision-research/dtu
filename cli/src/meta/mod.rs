use crate::progress::{FAIL_MARKER, SUCCESS_MARKER};
use clap::{self, Args, Subcommand};
use dtu::db::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[derive(Args)]
pub struct Meta {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the progress database contents
    #[command()]
    ShowProgress(ShowProgress),

    /// Manually clear the progress step
    #[command()]
    ClearProgress(ClearProgress),

    /// Manually set the progress step
    #[command()]
    SetProgress(SetProgress),
}

impl Meta {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::ShowProgress(c) => c.run(),
            Command::ClearProgress(c) => c.run(),
            Command::SetProgress(c) => c.run(),
        }
    }
}

#[derive(Args)]
struct ClearProgress {
    /// The step to chanage
    #[arg(short, long)]
    step: Prereq,
}

impl ClearProgress {
    fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = MetaSqliteDatabase::new(&ctx)?;
        db.update_prereq(self.step, false)?;
        Ok(())
    }
}

#[derive(Args)]
struct SetProgress {
    /// The step to chanage
    #[arg(short, long)]
    step: Prereq,
}

impl SetProgress {
    fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = MetaSqliteDatabase::new(&ctx)?;
        db.update_prereq(self.step, true)?;
        Ok(())
    }
}

#[derive(Args)]
struct ShowProgress {}

impl ShowProgress {
    fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = MetaSqliteDatabase::new(&ctx)?;
        let progress = db.get_all_progress()?;
        for p in &progress {
            let marker = if p.completed {
                SUCCESS_MARKER
            } else {
                FAIL_MARKER
            };

            println!("{} - {}", p.step, marker);
        }
        Ok(())
    }
}
