use clap::{self, Args};
use std::process::exit;

use crate::parsers::parse_intent_string;
use dtu::app::server::AppServer;
use dtu::db::sql::meta::models::AppActivity;
use dtu::db::sql::MetaDatabase;
use dtu::Context;

use crate::parsers::AppActivityValueParser;
use crate::printer::{color, Printer};
use crate::utils::get_app_server;

#[derive(Args)]
pub struct RunTest {
    /// The name of the class to remove
    #[arg(short, long, value_parser = AppActivityValueParser)]
    activity: AppActivity,

    /// Intent arguments that will be passed through
    ///
    /// The intent is specified similar to how a Parcel is specified (see{n}
    /// dtu call --help to see that DSL), but there is always a preceding{n}
    /// key. For example:{n}
    ///{n}
    /// -- "BOOL_KEY" z false "STR_KEY" str "string"
    #[arg(last = true)]
    intent: Vec<String>,
}

impl RunTest {
    pub fn run(&self, ctx: &dyn Context, _meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let name = self.activity.name.as_str();
        let intent = if self.intent.is_empty() {
            None
        } else {
            Some(parse_intent_string(self.intent.as_slice())?)
        };

        let mut srv = get_app_server(&ctx)?;
        let res = srv.run_test(name, intent.as_ref())?;

        let printer = Printer::new();

        for l in res.output.lines() {
            // 4 for "[X] " prefix
            if l.len() < 4 {
                printer.println("");
                continue;
            }
            let level = l.as_bytes()[1];
            let color = match level {
                b'D' => color::GREY,
                b'W' => color::YELLOW,
                b'I' => color::CYAN,
                b'E' => color::RED,
                _ => color::GREY,
            };
            printer.println_colored(&l[4..], color);
        }

        if !res.success {
            exit(1);
        }
        Ok(())
    }
}
