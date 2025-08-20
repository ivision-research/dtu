use clap::{self, Args};

use dtu::app::server::AppServer;
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

use crate::parsers::parse_intent_string;
use crate::utils::get_app_server;

#[derive(Args)]
pub struct Broadcast {
    /// The action to broadcast
    #[arg(short, long)]
    action: Option<String>,

    /// Data URI to send with the intent
    #[arg(short, long)]
    data: Option<String>,

    /// The component to send the broadcast to as `pkg/class`
    #[arg(short, long)]
    component: Option<String>,

    #[arg(short, long)]
    flags: Option<Vec<String>>,

    /// Intent arguments that will be passed through
    #[arg(last = true)]
    intent: Vec<String>,
}

impl Broadcast {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::AppSetup)?;

        let intent = if self.intent.is_empty() {
            None
        } else {
            Some(parse_intent_string(self.intent.as_slice())?)
        };

        let (pkg, class) = match self.component.as_ref() {
            Some(component) => {
                let (pkg, class) = component
                    .split_once('/')
                    .ok_or_else(|| anyhow::Error::msg(format!("bad component: {}", component)))?;
                (Some(pkg), Some(class))
            }
            None => (None, None),
        };

        let mut srv = get_app_server(&ctx)?;

        srv.broadcast(
            self.action.as_ref().map(|it| it.as_str()),
            self.data.as_ref().map(|it| it.as_str()),
            pkg,
            class,
            self.flags.as_ref(),
            intent.as_ref(),
        )?;

        Ok(())
    }
}
