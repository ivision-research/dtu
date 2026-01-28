use clap::{self, Args};
use dtu::db::device::schema::protected_broadcasts;
use dtu::db::DeviceDatabase;
use dtu::diesel::prelude::*;
use dtu::prereqs::Prereq;
use dtu::utils::ensure_prereq;
use dtu::Context;

#[derive(Args)]
pub struct ProtectedBroadcast {
    /// String that the broadcast name should contain (case sensitive)
    #[arg(short, long)]
    containing: String,
}

impl ProtectedBroadcast {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::SQLDatabaseSetup)?;

        let like = format!("%{}%", self.containing);

        let db = DeviceDatabase::new(ctx)?;
        let perms = db.with_connection(|c| {
            protected_broadcasts::table
                .filter(protected_broadcasts::name.like(like))
                .select(protected_broadcasts::name)
                .get_results::<String>(c)
        })?;
        for p in perms {
            println!("{}", p);
        }
        Ok(())
    }
}
