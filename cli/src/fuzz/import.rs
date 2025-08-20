use std::fs::File;

use clap::{self, Args, ValueHint};

use dtu::DefaultContext;
use dtu::tasks::fuzz;

#[derive(Args)]
pub struct Import {
    /// The file that holds the ssfuzz/fast CSV results
    #[arg(short, long, value_hint=ValueHint::FilePath)]
    file: String,
}

impl Import {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new(); 

        let mut file = File::open(&self.file)?;

        fuzz::parse_csv(&ctx, &mut file)?;

        Ok(())
    }
}
