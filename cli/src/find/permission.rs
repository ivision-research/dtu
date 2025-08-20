use clap::{self, Args};
use dtu::db::sql::{DeviceDatabase, DeviceSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::ensure_prereq;
use dtu::DefaultContext;

#[derive(Args)]
pub struct Permission {
    /// String that the permission should contain (case sensitive)
    #[arg(short, long)]
    containing: String,
}

impl Permission {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;

        let like = format!("%{}%", self.containing);

        let db = DeviceSqliteDatabase::new(&ctx)?;
        let perms = db.get_permissions_by_name_like(&like)?;
        for p in perms {
            println!("{}", p);
        }
        Ok(())
    }
}
