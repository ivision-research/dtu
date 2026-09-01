use std::collections::HashMap;

use clap::{self, Args};
use dtu::{
    analysis::{db::GraphTaintAnalysisDb, taint::TaintSource},
    db::{
        graph::{models::MethodId, GraphDatabase, MethodSpec},
        meta::get_default_metadb,
        ApkComponent, ApkIPC, DeviceDatabase, MetaDatabase,
    },
    prereqs::Prereq,
    smalisa::AccessFlag,
    utils::ClassName,
    Context,
};

use crate::taint::common::{analyze, methods_for_class, Analysis, ComponentOpts};

#[derive(Args)]
pub struct Providers {
    #[command(flatten)]
    opts: ComponentOpts,
}

struct Entrypoint {
    name: &'static str,
    signature: &'static str,
    /// The parameter registers worth following. `p0` is the receiver, and every parameter here is
    /// a single register, so these count up from `p1`.
    params: &'static [u16],
}

impl Entrypoint {
    const fn new(name: &'static str, signature: &'static str, params: &'static [u16]) -> Self {
        Self {
            name,
            signature,
            params,
        }
    }

    fn sources(&self) -> Vec<TaintSource> {
        self.params
            .iter()
            .map(|register| TaintSource::Param {
                register: *register,
            })
            .collect()
    }
}

const ENTRYPOINTS: &[Entrypoint] = &[
    Entrypoint::new(
        "query",
        "Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;",
        &[1, 2, 3, 4, 5],
    ),
    Entrypoint::new(
        "query",
        "Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;Landroid/os/CancellationSignal;",
        &[1, 2, 3, 4, 5],
    ),
    Entrypoint::new(
        "query",
        "Landroid/net/Uri;[Ljava/lang/String;Landroid/os/Bundle;Landroid/os/CancellationSignal;",
        &[1, 2, 3],
    ),
    Entrypoint::new(
        "insert",
        "Landroid/net/Uri;Landroid/content/ContentValues;",
        &[1, 2],
    ),
    Entrypoint::new(
        "insert",
        "Landroid/net/Uri;Landroid/content/ContentValues;Landroid/os/Bundle;",
        &[1, 2, 3],
    ),
    Entrypoint::new(
        "bulkInsert",
        "Landroid/net/Uri;[Landroid/content/ContentValues;",
        &[1, 2],
    ),
    Entrypoint::new(
        "update",
        "Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;",
        &[1, 2, 3, 4],
    ),
    Entrypoint::new(
        "update",
        "Landroid/net/Uri;Landroid/content/ContentValues;Landroid/os/Bundle;",
        &[1, 2, 3],
    ),
    Entrypoint::new(
        "delete",
        "Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;",
        &[1, 2, 3],
    ),
    Entrypoint::new("delete", "Landroid/net/Uri;Landroid/os/Bundle;", &[1, 2]),
    Entrypoint::new("openFile", "Landroid/net/Uri;Ljava/lang/String;", &[1]),
    Entrypoint::new(
        "openFile",
        "Landroid/net/Uri;Ljava/lang/String;Landroid/os/CancellationSignal;",
        &[1],
    ),
    Entrypoint::new("openAssetFile", "Landroid/net/Uri;Ljava/lang/String;", &[1]),
    Entrypoint::new(
        "openAssetFile",
        "Landroid/net/Uri;Ljava/lang/String;Landroid/os/CancellationSignal;",
        &[1],
    ),
    Entrypoint::new(
        "openTypedAssetFile",
        "Landroid/net/Uri;Ljava/lang/String;Landroid/os/Bundle;",
        &[1, 2, 3],
    ),
    Entrypoint::new(
        "openTypedAssetFile",
        "Landroid/net/Uri;Ljava/lang/String;Landroid/os/Bundle;Landroid/os/CancellationSignal;",
        &[1, 2, 3],
    ),
    Entrypoint::new(
        "call",
        "Ljava/lang/String;Ljava/lang/String;Landroid/os/Bundle;",
        &[1, 2, 3],
    ),
    Entrypoint::new(
        "call",
        "Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Landroid/os/Bundle;",
        &[1, 2, 3, 4],
    ),
    Entrypoint::new(
        "getStreamTypes",
        "Landroid/net/Uri;Ljava/lang/String;",
        &[1, 2],
    ),
    Entrypoint::new("applyBatch", "Ljava/util/ArrayList;", &[1]),
    Entrypoint::new(
        "applyBatch",
        "Ljava/lang/String;Ljava/util/ArrayList;",
        &[1, 2],
    ),
    Entrypoint::new(
        "refresh",
        "Landroid/net/Uri;Landroid/os/Bundle;Landroid/os/CancellationSignal;",
        &[1, 2],
    ),
];

impl Providers {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let db = GraphTaintAnalysisDb::new_from_path(ctx, &self.opts.run.out_file)?;

        analyze(ctx, db, &self.opts.run, |gdb| {
            let meta = get_default_metadb(ctx)?;
            meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
            let db = DeviceDatabase::new(ctx)?;

            let providers = self.opts.select(
                ctx,
                &meta,
                &db,
                || db.get_providers(),
                |id| db.get_provider_diffs_by_diff_id(id),
            )?;

            let mut methods: Vec<MethodSpec> = Vec::new();
            let mut seeds: HashMap<MethodId, Vec<TaintSource>> = HashMap::new();
            let mut without_entrypoints = 0usize;

            for provider in providers {
                let apk = db.get_apk_by_id(provider.get_apk_id())?;
                let source = apk.device_path.as_squashed_str();
                let class = provider.get_class_name();

                let found = match self.resolve_entrypoints(gdb, &class, source) {
                    Ok(found) => found,
                    Err(e) => {
                        without_entrypoints += 1;
                        log::warn!("failed to resolve entrypoints for {class}: {e}");
                        continue;
                    }
                };

                if found.is_empty() {
                    without_entrypoints += 1;
                    log::warn!("{class} implements no ContentProvider entrypoints");
                    continue;
                }

                for (method, entrypoint) in found {
                    seeds.insert(method.id, entrypoint.sources());
                    methods.push(method);
                }
            }

            log::info!(
                "{} provider entrypoints seeded, {} providers had none",
                methods.len(),
                without_entrypoints
            );

            if methods.is_empty() {
                anyhow::bail!("no provider entrypoints to analyze, run with -v for the reasons");
            }

            Ok(Analysis::new(methods, seeds))
        })?;
        Ok(())
    }

    fn resolve_entrypoints(
        &self,
        gdb: &dyn GraphDatabase,
        class: &ClassName,
        source: &str,
    ) -> anyhow::Result<Vec<(MethodSpec, &'static Entrypoint)>> {
        const NO_BODY: AccessFlag = AccessFlag::ABSTRACT.union(AccessFlag::NATIVE);

        // methods_for_class returns the class before its parents, so the first match wins
        let implemented = methods_for_class(gdb, class, source)?;

        let mut out = Vec::new();
        for entrypoint in ENTRYPOINTS {
            let found = implemented.iter().find(|it| {
                it.name == entrypoint.name
                    && it.signature == entrypoint.signature
                    && !it.access_flags.intersects(NO_BODY)
            });
            if let Some(method) = found {
                out.push((method.clone(), entrypoint));
            }
        }
        Ok(out)
    }
}
