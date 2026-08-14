use clap::{self, Args};
use dtu::{prereqs::Prereq, utils::ensure_prereq, DefaultContext};

mod activity_intents;
mod common;
mod filter;
mod methods;
mod providers;
mod receiver_intents;
mod service_binders;
mod system_services;

use activity_intents::ActivityIntents;
use filter::{FilterField, FilterMethod};
use methods::Methods;
use providers::Providers;
use receiver_intents::ReceiverIntents;
use service_binders::ServiceBinders;
use system_services::SystemServices;

#[derive(Args)]
pub struct Taint {
    #[command(subcommand)]
    command: Subcommand,
}

#[derive(clap::Subcommand)]
enum Subcommand {
    /// Traces all interesting data read off of the received Intent
    ReceiverIntents(ReceiverIntents),
    /// Traces data coming out of getIntent for Activities
    ActivityIntents(ActivityIntents),
    /// Traces input parameters of returned IBinders from app services. Note that this is not
    /// comprehensive!
    ServiceBinders(ServiceBinders),
    /// Traces input parameters of system service methods
    SystemServices(SystemServices),
    /// Traces data entering ContentProviders
    Providers(Providers),
    /// Taint analysis on arbitrary methods
    Methods(Methods),
    /// Filter a full taint report to only those that reach a provided method
    FilterMethod(FilterMethod),
    /// Filter a full taint report to only those that reach a provided field
    FilterField(FilterField),
}

impl Taint {
    pub fn run(self) -> anyhow::Result<()> {
        // These don't need the database or context, they just need a valid input
        let cmd = match self.command {
            Subcommand::FilterMethod(c) => return c.run(),
            Subcommand::FilterField(c) => return c.run(),
            _ => self.command,
        };

        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::GraphDatabaseSetup)?;

        match cmd {
            Subcommand::ReceiverIntents(c) => c.run(&ctx),
            Subcommand::ActivityIntents(c) => c.run(&ctx),
            Subcommand::ServiceBinders(c) => c.run(&ctx),
            Subcommand::SystemServices(c) => c.run(&ctx),
            Subcommand::Providers(c) => c.run(&ctx),
            Subcommand::Methods(c) => c.run(&ctx),
            Subcommand::FilterMethod(_) => unreachable!("handled above"),
            Subcommand::FilterField(_) => unreachable!("handled above"),
        }
    }
}
