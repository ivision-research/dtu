use clap::{self, Args, Subcommand};

use super::import::Import;
use super::logcat::Logcat;
use super::unprotected::Unprotected;

#[derive(Args)]
pub struct Fuzz {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse the CSV output from ssfuzz/fast
    #[command()]
    Import(Import),

    /// Get service endpoints that did not throw a security exception
    #[command()]
    Unprotected(Unprotected),

    /// Setup logcat listeners
    #[command()]
    Logcat(Logcat),
}

impl Fuzz {
    pub fn run(&self) -> anyhow::Result<()> {
        match &self.command {
            Commands::Import(c) => c.run(),
            Commands::Unprotected(c) => c.run(),
            Commands::Logcat(c) => c.run(),
        }
    }
}
