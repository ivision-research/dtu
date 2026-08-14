use std::collections::HashMap;

use clap::{self, Args};
use dtu::{
    analysis::taint::{TaintSeedOptions, TaintSource},
    db::{
        graph::{get_default_graphdb, models::MethodId, MethodSpec, FRAMEWORK_SOURCE},
        meta::get_default_metadb,
        ApkComponent, ApkIPC, DeviceDatabase, MetaDatabase,
    },
    prereqs::Prereq,
    utils::{ClassName, Denylist},
    Context,
};

use crate::cache_key;
use crate::taint::common::{analyze, methods_for_class, Analysis, ComponentOpts};

/// Classes that call `getIntent` as part of the framework's own plumbing
///
/// These are reachable from every activity, so without this each one shows up in every report.
const NEVER_SEED: &[&str] = &[
    "Landroid/app/Instrumentation;",
    "Landroid/app/Activity;",
    "Landroid/content/ClipData;",
    "Landroid/app/RemoteInput;",
];

#[derive(Args)]
pub struct ActivityIntents {
    #[command(flatten)]
    opts: ComponentOpts,
}

impl ActivityIntents {
    pub fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        let gdb = get_default_graphdb(ctx)?;
        let cache = cache_key!("analysis-activity-intents", &self.opts.hash_key());

        let report = analyze(ctx, &gdb, &self.opts.run, &cache, || {
            let meta = get_default_metadb(ctx)?;
            meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
            let db = DeviceDatabase::new(ctx)?;

            let activities = self.opts.select(
                ctx,
                &meta,
                &db,
                || db.get_activities(),
                |id| db.get_activity_diffs_by_diff_id(id),
            )?;

            let mut methods: Vec<MethodSpec> = Vec::new();
            let mut seeds: HashMap<MethodId, Vec<TaintSource>> = HashMap::new();
            let mut sources = vec![String::from(FRAMEWORK_SOURCE)];

            for activity in activities {
                let apk = db.get_apk_by_id(activity.get_apk_id())?;
                let source = apk.device_path.as_squashed_str();
                if !sources.iter().any(|it| it == source) {
                    sources.push(source.to_string());
                }

                for method in methods_for_class(&gdb, &activity.get_class_name(), source)? {
                    // Most call sites name Landroid/app/Activity; but subclasses that declare
                    // their own getIntent are named directly, so the class can't be pinned. The
                    // cost is the unrelated getIntent methods that also return an Intent
                    let mut for_method = vec![TaintSource::MethodCall {
                        class: None,
                        method: "getIntent".into(),
                        args: "".into(),
                        ret: Some("Landroid/content/Intent;".into()),
                    }];
                    if method.name == "onNewIntent" {
                        for_method.push(TaintSource::Param { register: 1 });
                    }
                    seeds.insert(method.id, for_method);
                    methods.push(method);
                }
            }

            let mut deny_classes = Denylist::new();
            deny_classes.extend(NEVER_SEED.iter().map(|it| ClassName::from(*it)));

            Ok(
                Analysis::new(methods, seeds).with_seed_options(TaintSeedOptions {
                    sources,
                    deny_classes: Some(deny_classes),
                }),
            )
        })?;

        println!("{}", serde_json::to_string(&report)?);
        Ok(())
    }
}
