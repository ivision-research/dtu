use std::collections::HashMap;

use clap::{self, Args};
use dtu::{
    analysis::{taint::TaintSource, typing::complex_param_sources},
    db::{
        device::models::DiffSource,
        graph::{
            get_default_graphdb, models::MethodId, GraphDatabase, MethodSearch, MethodSearchParams,
            MethodSpec,
        },
        meta::get_default_metadb,
        DeviceDatabase, Diffable, MetaDatabase,
    },
    prereqs::Prereq,
    Context,
};

use crate::cache_key;
use crate::diff::get_diff_source;
use crate::parsers::DiffSourceValueParser;
use crate::taint::common::{analyze, Analysis, RunOpts};
use crate::utils::{bool_hash_key, opt_asref_hash_key, opt_diff_hash_key};

#[derive(Args)]
pub struct SystemServices {
    #[command(flatten)]
    run: RunOpts,

    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
    only_new: bool,

    /// Set the diff source (only valid with -n/--only-new) otherwise the emulator is the default
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,

    /// Only analyze the named service
    #[arg(short = 'N', long)]
    name: Option<String>,
}

impl SystemServices {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let gdb = get_default_graphdb(ctx)?;
        let cache = cache_key!(
            "analysis-system-services",
            bool_hash_key(self.only_new),
            opt_diff_hash_key(&self.diff_source),
            opt_asref_hash_key(&self.name),
            &self.run.hash_key()
        );

        let report = analyze(ctx, &gdb, &self.run, &cache, || {
            let meta = get_default_metadb(ctx)?;
            meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
            let db = DeviceDatabase::new(ctx)?;

            let services = self.select(ctx, &meta, &db)?;
            let mut methods: Vec<MethodSpec> = Vec::new();
            let mut seeds: HashMap<MethodId, Vec<TaintSource>> = HashMap::new();
            let mut without_impl = 0usize;

            for service in services {
                let transactions = db.get_system_service_methods_by_service_id(service.id)?;
                let impls = db.get_system_service_impls(service.id)?;
                if impls.is_empty() {
                    if !transactions.is_empty() {
                        without_impl += 1;
                        log::warn!(
                            "system service {} has {} transactions but no known implementation",
                            service.name,
                            transactions.len()
                        );
                    }
                    continue;
                }

                for imp in impls {
                    for transaction in &transactions {
                        let Some(signature) = transaction.signature.as_deref() else {
                            log::warn!(
                                "{}->{} has no recorded signature, skipping it",
                                service.name,
                                transaction.name
                            );
                            continue;
                        };
                        let params = match complex_param_sources(signature) {
                            Ok(v) if v.is_empty() => continue,
                            Ok(v) => v,
                            Err(e) => {
                                log::warn!(
                                    "{}->{} has an unparseable signature {signature}: {e}",
                                    service.name,
                                    transaction.name
                                );
                                continue;
                            }
                        };

                        let search = MethodSearch::new(
                            MethodSearchParams::ByFullSpec {
                                class: &imp.class_name,
                                name: &transaction.name,
                                signature,
                            },
                            Some(imp.source.as_str()),
                            None,
                        );
                        let found = gdb.get_methods(&search)?;
                        if found.is_empty() {
                            log::warn!(
                                "{}->{}({signature}) is not implemented by {}",
                                service.name,
                                transaction.name,
                                imp.class_name
                            );
                            continue;
                        }
                        for method in found {
                            seeds.insert(method.id, params.clone());
                            methods.push(method);
                        }
                    }
                }
            }

            log::info!(
                "{} transaction methods seeded, {} services had no implementation",
                methods.len(),
                without_impl
            );

            if methods.is_empty() {
                anyhow::bail!("no transaction methods to analyze, run with -l 1 for the reasons");
            }

            Ok(Analysis::new(methods, seeds))
        })?;

        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    }

    fn select(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &DeviceDatabase,
    ) -> anyhow::Result<Vec<dtu::db::device::models::SystemService>> {
        let services = if self.only_new {
            let source = get_diff_source(ctx, meta, db, &self.diff_source)?;
            db.get_system_service_diffs_by_diff_id(source.id)?
                .into_iter()
                .filter(|it| !it.in_diff())
                .map(|it| it.service)
                .collect::<Vec<_>>()
        } else {
            db.get_system_services()?
        };

        Ok(match &self.name {
            Some(name) => services.into_iter().filter(|it| it.name == *name).collect(),
            None => services,
        })
    }
}
