use std::io::stdout;

use clap::Args;
use dtu::db::device::models::{
    DiffSource, SimpleSystemServiceMethod, SystemService, SystemServiceMethod,
};
use dtu::db::meta::get_default_metadb;
use dtu::db::{DeviceDatabase, MetaDatabase};
use dtu::prereqs::Prereq;
use dtu::{Context, DefaultContext};
use itertools::Itertools;

use crate::cache_key;
use crate::diff::get_diff_source;
use crate::parsers::{DiffSourceValueParser, SystemServiceValueParser};
use crate::utils::{bool_hash_key, inum_hash_key, opt_diff_hash_key, project_cacheable};

#[derive(Args)]
pub struct SystemServiceMethods {
    /// The service to get the methods for
    #[arg(short, long, value_parser = SystemServiceValueParser)]
    service: SystemService,

    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
    only_new: bool,

    /// Set the diff source (only valid with -n/--only-new) otherwise the emulator is the default
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,

    /// Print the results as JSON
    #[arg(short, long)]
    json: bool,
}

impl SystemServiceMethods {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx)?;
        meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
        let db = DeviceDatabase::new(&ctx)?;

        let cache_key = cache_key!(
            "list-system-service-methods",
            &inum_hash_key(self.service.id),
            bool_hash_key(self.only_new),
            opt_diff_hash_key(&self.diff_source)
        );

        let methods = project_cacheable(&ctx, &cache_key, false, || {
            self.get_methods(&ctx, &meta, &db)
        })?;

        if self.json {
            serde_json::to_writer(stdout(), &methods)?;
            return Ok(());
        }

        for m in methods {
            println!("{} - {}({}): {}", m.txn_id, m.name, m.args, m.ret);
        }

        Ok(())
    }

    fn get_methods(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &DeviceDatabase,
    ) -> anyhow::Result<Vec<SimpleSystemServiceMethod>> {
        let all_methods = if self.only_new {
            let diff = get_diff_source(ctx, meta, db, &self.diff_source)?;
            let methods =
                db.get_system_service_method_diffs_for_service(self.service.id, diff.id)?;
            methods
                .into_iter()
                .filter_map(|it| {
                    if it.exists_in_diff {
                        None
                    } else {
                        Some(SimpleSystemServiceMethod::from(it.method))
                    }
                })
                .sorted_by(|lhs: &SimpleSystemServiceMethod, rhs| lhs.txn_id.cmp(&rhs.txn_id))
                .collect::<Vec<_>>()
        } else {
            db.get_system_service_methods_by_service_id(self.service.id)?
                .into_iter()
                .sorted_by(|lhs: &SystemServiceMethod, rhs| {
                    lhs.transaction_id.cmp(&rhs.transaction_id)
                })
                .map(SimpleSystemServiceMethod::from)
                .collect::<Vec<_>>()
        };
        Ok(all_methods)
    }
}
