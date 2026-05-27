use std::io;

use clap::{self, Args};
use dtu::db::device::schema::{apks, permissions};
use dtu::db::DeviceDatabase;
use dtu::diesel::prelude::*;
use dtu::prereqs::Prereq;
use dtu::utils::ensure_prereq;
use dtu::Context;

#[derive(Args)]
pub struct Permission {
    /// String that the permission should contain (case sensitive)
    #[arg(short, long)]
    containing: String,

    #[arg(short, long)]
    json: bool,
}

impl Permission {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::SQLDatabaseSetup)?;

        let like = format!("%{}%", self.containing);

        let db = DeviceDatabase::new(ctx)?;

        let q = permissions::table
            .inner_join(apks::table)
            .select((apks::name, permissions::name, permissions::protection_level))
            .filter(permissions::name.like(&like));

        if self.json {
            #[derive(serde::Serialize)]
            struct JsonOutput {
                apk: String,
                name: String,
                level: String,
            }

            let results = db
                .with_connection(|c| q.get_results::<(String, String, String)>(c))?
                .into_iter()
                .map(|(apk, name, level)| JsonOutput { apk, name, level })
                .collect::<Vec<JsonOutput>>();
            serde_json::to_writer(io::stdout(), &results)?;
        } else {
            let results = db.with_connection(|c| q.get_results::<(String, String, String)>(c))?;

            for (apk, perm, level) in results {
                println!("{apk} | {perm} - {level}");
            }
        }
        Ok(())
    }
}
