use clap::{self, Args};

use dtu::app::server::AppServer;
use dtu::db::sql::{MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

use crate::parsers::parse_intent_string;
use crate::utils::get_app_server;

#[derive(Args)]
pub struct StartService {
    /// An action to include with the Intent
    #[arg(short, long)]
    action: Option<String>,

    /// The service to start as `pkg/class`
    #[arg(short, long)]
    component: Option<String>,

    /// Data URI to send with the intent
    #[arg(short, long)]
    data: Option<String>,

    /// Intent arguments that will be passed through
    #[arg(last = true)]
    intent: Vec<String>,
}

impl StartService {
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

        srv.start_service(
            self.action.as_ref().map(|it| it.as_str()),
            self.data.as_ref().map(|it| it.as_str()),
            pkg,
            class,
            None,
            intent.as_ref(),
        )?;

        Ok(())
    }
}
