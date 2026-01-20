use clap::{self, Args};
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::ensure_prereq;
use dtu::Context;

#[derive(Args)]
pub struct Permission {
    /// String that the permission should contain (case sensitive)
    #[arg(short, long)]
    containing: String,
}

impl Permission {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        ensure_prereq(ctx, Prereq::SQLDatabaseSetup)?;

        let like = format!("%{}%", self.containing);

        let db = DeviceSqliteDatabase::new(ctx)?;
        let perms = db.get_permissions_by_name_like(&like)?;
        for p in perms {
            println!("{}", p);
        }
        Ok(())
    }
}
