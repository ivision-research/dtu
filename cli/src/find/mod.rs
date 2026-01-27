use clap::{self, Args, Subcommand};

mod service_file;
use dtu::{
    db::graph::{get_default_graphdb, DefaultGraphDatabase},
    prereqs::Prereq,
    utils::ensure_prereq,
    Context, DefaultContext,
};
use service_file::ServiceFile;

mod smali_file;
use smali_file::SmaliFile;

mod permission;
use permission::Permission;

mod protected_broadcast;
use protected_broadcast::ProtectedBroadcast;

mod utils;

mod callers;
use callers::FindCallers;

mod apk_graph;
use apk_graph::{ApkIPCCallsGeneric, FindIPCCalls, FindIntentActivities, FindParseUri};
#[derive(Args)]
pub struct Find {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find service related smali files
    #[command()]
    ServiceFile(ServiceFile),

    /// Find a protected broadcast
    #[command()]
    ProtectedBroadcast(ProtectedBroadcast),

    /// Find a permission
    #[command()]
    Permission(Permission),

    /// Find a smali file
    #[command()]
    SmaliFile(SmaliFile),

    /// Find classes that call the given method
    #[command()]
    Callers(FindCallers),

    /// Find Activitys that call `getIntent`
    #[command()]
    IntentActivities(FindIntentActivities),

    /// Find IPC that calls Intent.parseUri
    #[command()]
    ParseUri(FindParseUri),

    /// Find calls leaving IPC to the given method
    #[command()]
    IPCCalls(FindIPCCalls),
}

fn graph_db(ctx: &dyn Context) -> anyhow::Result<DefaultGraphDatabase> {
    ensure_prereq(ctx, Prereq::GraphDatabaseSetup)?;
    let db = get_default_graphdb(&ctx)?;
    Ok(db)
}

impl Find {
    pub fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        match self.command {
            Command::ServiceFile(c) => c.run(&ctx),
            Command::Permission(c) => c.run(&ctx),
            Command::ProtectedBroadcast(c) => c.run(&ctx),
            Command::SmaliFile(c) => c.run(&ctx),

            Command::Callers(c) => {
                let db = graph_db(&ctx)?;
                c.run(&db)
            }
            Command::IntentActivities(c) => {
                let db = graph_db(&ctx)?;
                c.run(&ctx, &db)
            }
            Command::ParseUri(c) => {
                let db = graph_db(&ctx)?;
                let generic = ApkIPCCallsGeneric::from(c);
                generic.run(&ctx, &db)
            }
            Command::IPCCalls(c) => {
                let db = graph_db(&ctx)?;
                let generic = ApkIPCCallsGeneric::from(c);
                generic.run(&ctx, &db)
            }
        }
    }
}
