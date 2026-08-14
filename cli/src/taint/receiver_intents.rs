use clap::{self, Args};
use dtu::{
    analysis::taint::TaintSource,
    db::{
        graph::{GraphDatabase, MethodSearch, MethodSearchParams, MethodSpec},
        meta::get_default_metadb,
        ApkComponent, ApkIPC, DeviceDatabase, MetaDatabase,
    },
    prereqs::Prereq,
    Context,
};

use crate::cache_key;
use crate::taint::common::{analyze, Analysis, ComponentOpts};

#[derive(Args)]
pub struct ReceiverIntents {
    #[command(flatten)]
    opts: ComponentOpts,
}

impl ReceiverIntents {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let gdb = dtu::db::graph::get_default_graphdb(ctx)?;
        let cache = cache_key!("analysis-receiver-intents", &self.opts.hash_key());

        let report = analyze(ctx, &gdb, &self.opts.run, &cache, || {
            // We track the passed in intent. The taint rules will cover all interesting methods and
            // propagation of the intent into other methods for us. This is better than choosing a
            // list of "interesting methods" because hey who knows what's interesting and it also
            // future proofs.
            let p2 = TaintSource::Param { register: 2 };

            let meta = get_default_metadb(ctx)?;
            meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
            let db = DeviceDatabase::new(ctx)?;

            let receivers = self.opts.select(
                ctx,
                &meta,
                &db,
                || db.get_receivers(),
                |id| db.get_receiver_diffs_by_diff_id(id),
            )?;

            let mut methods: Vec<MethodSpec> = Vec::new();
            for rx in receivers {
                let apk = db.get_apk_by_id(rx.get_apk_id())?;
                let class = rx.get_class_name();
                let search = MethodSearch::new(
                    MethodSearchParams::ByFullSpec {
                        class: &class,
                        name: "onReceive",
                        signature: "Landroid/content/Context;Landroid/content/Intent;",
                    },
                    Some(apk.device_path.as_squashed_str()),
                    None,
                );
                methods.extend(gdb.get_methods(&search)?);
            }

            Ok(Analysis::new(methods, vec![p2]))
        })?;

        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    }
}
