use clap::{self, Args};
use dtu::{prereqs::Prereq, utils::ensure_prereq, DefaultContext};

mod activity_intents;
mod common;
mod methods;
mod providers;
mod filter;
mod receiver_intents;
mod service_binders;
mod sqlite;
mod system_services;
mod ui;

use ui::Ui;
use activity_intents::ActivityIntents;
use methods::Methods;
use providers::Providers;
use filter::Filter;
use receiver_intents::ReceiverIntents;
use service_binders::ServiceBinders;
use sqlite::Sqlite;
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
    /// Display filtered views of the database as JSON
    ///
    /// The following filters are available:
    ///
    /// class=CLASS     - Searches for CLASS in classes in any sink{n}
    /// method=METHOD   - Searches for METHOD in method in any sink{n}
    /// field=FIELD     - Searches for FIELD in field names of any sink{n}
    /// args=ARGS       - Searches for ARGS in the signature of any sink{n}
    /// ret=RET         - Searches for RET in the return type of any sink{n}
    /// nophi           - Searches for routes that had no phi nodes{n}
    /// min-len=X       - Searches for routes with minimum length X{n}
    /// max-len=X       - Searches for routes with maximum length X{n}
    /// {n}
    /// Filters can be combined, for example class=Intent method=getString would{n}
    /// match `Landroid/os/[Intent];->[getString]Extra(...)`{n}
    /// {n}
    /// The output is always a JSON object with `results` and `map` as the two{n}
    /// base keys. The map can be used to resolve any value in `results`.
    Filter(Filter),
    /// Launch an SQLite repl with the graph database attached and temporary views
    Sqlite(Sqlite),
    Ui(Ui),
}

impl Taint {
    pub fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::GraphDatabaseSetup)?;

        match self.command {
            Subcommand::ReceiverIntents(c) => c.run(&ctx),
            Subcommand::ActivityIntents(c) => c.run(&ctx),
            Subcommand::ServiceBinders(c) => c.run(&ctx),
            Subcommand::SystemServices(c) => c.run(&ctx),
            Subcommand::Providers(c) => c.run(&ctx),
            Subcommand::Methods(c) => c.run(&ctx),
            Subcommand::Filter(c) => c.run(&ctx),
            Subcommand::Sqlite(c) => c.run(&ctx),
            Subcommand::Ui(c) => c.run(&ctx),
        }
    }
}
