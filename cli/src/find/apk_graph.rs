use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::io::stdout;

use clap::{self, Args};
use dtu::db::device::models::Activity;
use dtu::db::device::schema::{activities, apks, providers, receivers, services};
use itertools::Itertools;
use sha2::{Digest, Sha256};

use crate::find::utils::get_method_search;
use crate::parsers::DevicePathValueParser;
use crate::printer::{color, Printer};
use crate::utils::{oshash, ostr, project_cacheable};
use dtu::db::graph::models::MethodCallPath;
use dtu::db::graph::GraphDatabase;
use dtu::db::{DeviceDatabase, Enablable, Exportable};
use dtu::diesel::prelude::*;
use dtu::utils::{hex, ClassName, DevicePath};
use dtu::Context;

/// Generic used to search for any ApkIPC call
pub struct ApkIPCCallsGeneric {
    apk: Option<DevicePath>,
    no_cache: bool,
    json: bool,
    depth: usize,
    cache: String,

    only_exported: bool,
    only_enabled: bool,

    method: Option<String>,
    signature: Option<String>,
    class: Option<ClassName>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SourcedClass<'a> {
    class: ClassName,
    source: Cow<'a, str>,
}

struct IPCClasses {
    apk_path: DevicePath,
    classes: Vec<ClassName>,
}

impl ApkIPCCallsGeneric {
    fn run_inner_source(
        &self,
        graphdb: &dyn GraphDatabase,
        source: Option<&str>,
        map: BTreeMap<i32, IPCClasses>,
        results: &mut ApkCallsResult,
    ) -> anyhow::Result<()> {
        let search = get_method_search(
            ostr(&self.method),
            self.class.as_ref(),
            ostr(&self.signature),
            source,
        )?;

        let mut lookup = HashMap::new();

        // Find all callers of the given method regardless of whether they are IPC or not
        let paths = graphdb.find_callers(&search, None, self.depth)?;

        for mpath in paths.into_iter() {
            if mpath.is_empty() {
                continue;
            }
            let key = SourcedClass {
                class: mpath.must_get_src_class().clone(),
                source: Cow::Owned(mpath.must_get_source().into()),
            };
            let value = mpath.path;
            lookup.insert(key, value);
        }

        for apk_classes in map.into_values() {
            let source = apk_classes.apk_path.as_squashed_str();
            for class in apk_classes.classes {
                let search = SourcedClass {
                    source: Cow::Borrowed(&source),
                    class,
                };

                if let Some(path) = lookup.get(&search) {
                    let devpath = DevicePath::from_squashed(source);
                    let found = MethodCallPath { path: path.clone() };
                    match results.get_mut(&devpath) {
                        Some(v) => v.push(found),
                        None => {
                            let v = vec![found];
                            results.insert(devpath, v);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn run_inner(
        &self,
        ctx: &dyn Context,
        graphdb: &dyn GraphDatabase,
    ) -> anyhow::Result<ApkCallsResult> {
        let db = DeviceDatabase::new(ctx)?;
        let source = self.apk.as_ref().map(|it| it.as_squashed_str());
        let mut apk_map = HashMap::new();
        for apk in db.get_apks()?.into_iter() {
            apk_map.insert(apk.id, apk.device_path);
        }

        // We handle only_exported/only_enabled with `eq_any` filters:
        //
        // if true, &[true, true] filters only on true
        // if false, &[false, true] will get either
        //
        // slightly less efficient, but much easier to write..

        let en_filter = &[self.only_enabled, true];
        let ex_filter = &[self.only_exported, true];

        let classes = db.with_connection(|c| {
            receivers::table
                .inner_join(apks::table)
                .select((apks::id, receivers::class_name))
                .filter(receivers::exported.eq_any(ex_filter))
                .filter(receivers::enabled.eq_any(en_filter))
                .union_all(
                    services::table
                        .inner_join(apks::table)
                        .select((apks::id, services::class_name))
                        .filter(services::exported.eq_any(ex_filter))
                        .filter(services::enabled.eq_any(en_filter)),
                )
                .union_all(
                    activities::table
                        .inner_join(apks::table)
                        .select((apks::id, activities::class_name))
                        .filter(activities::exported.eq_any(ex_filter))
                        .filter(activities::enabled.eq_any(en_filter)),
                )
                .union_all(
                    providers::table
                        .inner_join(apks::table)
                        .select((apks::id, providers::name))
                        .filter(providers::exported.eq_any(ex_filter))
                        .filter(providers::enabled.eq_any(en_filter)),
                )
                .get_results::<(i32, ClassName)>(c)
        })?;

        let apk_ids = classes.iter().map(|it| it.0).unique();

        let mut map = db
            .with_connection(|c| {
                apks::table
                    .select((apks::id, apks::device_path))
                    .filter(apks::id.eq_any(apk_ids))
                    .get_results::<(i32, DevicePath)>(c)
            })?
            .into_iter()
            .map(|(id, apk_path)| {
                (
                    id,
                    IPCClasses {
                        apk_path,
                        classes: Vec::new(),
                    },
                )
            })
            .collect::<BTreeMap<i32, IPCClasses>>();

        for (apk, class) in classes {
            if let Some(v) = map.get_mut(&apk) {
                v.classes.push(class);
            }
        }

        let mut results = ApkCallsResult::new();

        self.run_inner_source(graphdb, source, map, &mut results)?;

        Ok(results)
    }

    pub fn run(&self, ctx: &dyn Context, graphdb: &dyn GraphDatabase) -> anyhow::Result<()> {
        let create = || self.run_inner(ctx, graphdb);

        let tag = bits_tag(self.only_exported, self.only_enabled);

        let cache_file_name = match &self.apk {
            None => format!("all-{}-{}-{}", self.cache, self.depth, tag),
            Some(v) => format!(
                "{}-{}-{}-{}",
                v.as_squashed_str_no_ext(),
                self.cache,
                self.depth,
                tag
            ),
        };

        let res = project_cacheable(ctx, &cache_file_name, self.no_cache, create)?;
        dump_apk_calls_result(&res, self.json, self.depth > 1)
    }
}

fn bits_tag(only_exported: bool, only_enabled: bool) -> &'static str {
    if only_enabled {
        if only_exported {
            "b3"
        } else {
            "b2"
        }
    } else if only_exported {
        "b1"
    } else {
        "b0"
    }
}

impl From<FindIPCCalls> for ApkIPCCallsGeneric {
    fn from(value: FindIPCCalls) -> Self {
        let mut hasher = Sha256::new();
        oshash(&mut hasher, &value.name);
        oshash(&mut hasher, &value.signature);
        oshash(&mut hasher, &value.class);
        let res = hasher.finalize();
        let hex = hex::bytes_to_hex(&res);
        let cache = format!("ipc-calls-{}", hex);

        Self {
            apk: value.apk,
            no_cache: value.no_cache,
            json: value.json,
            depth: value.depth,
            only_exported: value.only_exported,
            only_enabled: value.only_enabled,
            cache,
            method: value.name,
            class: value.class,
            signature: value.signature,
        }
    }
}

impl From<FindParseUri> for ApkIPCCallsGeneric {
    fn from(value: FindParseUri) -> Self {
        Self {
            apk: value.apk,
            no_cache: value.no_cache,
            json: value.json,
            depth: 1,
            only_exported: value.only_exported,
            only_enabled: value.only_enabled,
            cache: "ipc-ParseUri".into(),
            method: Some(String::from("parseUri")),
            class: Some(ClassName::new("Landroid/content/Intent;".into())),
            signature: Some(String::from("Ljava/lang/String;I")),
        }
    }
}

#[derive(Args)]
pub struct FindParseUri {
    /// The APK to search, otherwise all APKs and the framework are searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Ignore the cached results
    #[arg(long, default_value_t = false)]
    no_cache: bool,

    /// Output JSON
    #[arg(short, long, default_value_t = false)]
    json: bool,

    /// Only show exported IPC
    #[arg(short = 'X', long, default_value_t = false)]
    only_exported: bool,

    /// Only show enabled IPC
    #[arg(short = 'E', long, default_value_t = false)]
    only_enabled: bool,
}

#[derive(Args)]
pub struct FindIntentActivities {
    /// The APK to search, otherwise all APKs and the framework are searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Ignore the cached results
    #[arg(long, default_value_t = false)]
    no_cache: bool,

    /// Output JSON
    #[arg(short, long, default_value_t = false)]
    json: bool,

    /// Strictly search for calls to Landroid/app/Activity;->getIntent()
    ///
    /// Ideally, this would always be true, but sometimes you'll see calls
    /// to getIntent that perform the same functionality but have a different
    /// target class
    #[arg(short, long, default_value_t = false)]
    strict: bool,
}

impl FindIntentActivities {
    fn run_inner(
        &self,
        ctx: &dyn Context,
        graphdb: &dyn GraphDatabase,
    ) -> anyhow::Result<ApkCallsResult> {
        let db = DeviceDatabase::new(ctx)?;
        let source = self.apk.as_ref().map(|it| it.as_squashed_str());
        let activity_class = ClassName::new("Landroid/app/Activity;".into());
        let mut results = ApkCallsResult::new();

        let search = get_method_search(
            Some("getIntent"),
            if self.strict {
                Some(&activity_class)
            } else {
                None
            },
            Some(""),
            source,
        )?;

        let activities = db
            .get_activities()?
            .into_iter()
            .filter(|it| it.is_enabled() && it.is_exported())
            .collect::<Vec<Activity>>();

        let mut apk_map = HashMap::new();
        for apk in db.get_apks()?.into_iter() {
            apk_map.insert(apk.id, apk.device_path);
        }

        let mpaths = graphdb.find_callers(&search, None, 1)?;

        if log::log_enabled!(log::Level::Trace) {
            for path in &mpaths {
                if path.is_empty() {
                    continue;
                }
                log::trace!(
                    "Caller: {} in {}",
                    path.must_get_src_class(),
                    path.must_get_source()
                );
            }
        }

        let mut lookup = HashMap::new();
        for mpath in mpaths.into_iter() {
            let key = SourcedClass {
                class: mpath.must_get_src_class().clone(),
                source: Cow::Owned(mpath.must_get_source().into()),
            };
            let value = mpath.path;
            lookup.insert(key, value);
        }

        for act in activities {
            log::debug!("Checking activity: {}", act);
            let source = apk_map
                .get(&act.apk_id)
                .map(|it| it.as_squashed_str())
                .unwrap_or("framework");
            let search = SourcedClass {
                class: act.class_name.clone(),
                source: Cow::Borrowed(source),
            };
            if let Some(path) = lookup.get(&search) {
                log::trace!("Activity {} is a caller", act);
                let devpath = DevicePath::from_squashed(source);
                let found = MethodCallPath { path: path.clone() };
                match results.get_mut(&devpath) {
                    Some(v) => v.push(found),
                    None => {
                        let v = vec![found];
                        results.insert(devpath, v);
                    }
                }
            }
        }
        Ok(results)
    }

    pub fn run(&self, ctx: &dyn Context, graphdb: &dyn GraphDatabase) -> anyhow::Result<()> {
        let create = || self.run_inner(ctx, graphdb);

        let mut cache_file_name = match &self.apk {
            None => Cow::Borrowed("all-intent-activities"),
            Some(v) => Cow::Owned(format!("{}-intent-activities", v.as_squashed_str_no_ext())),
        };

        if self.strict {
            cache_file_name = Cow::Owned(format!("{}-strict", cache_file_name));
        }

        let res = project_cacheable(ctx, &cache_file_name, self.no_cache, create)?;
        dump_apk_calls_result(&res, self.json, false)
    }
}

#[derive(Args)]
pub struct FindIPCCalls {
    /// The APK to search, otherwise all APKs are searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Method name
    #[arg(short, long)]
    name: Option<String>,

    /// Method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// Method class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Only show exported IPC
    #[arg(short = 'X', long, default_value_t = false)]
    only_exported: bool,

    /// Only show enabled IPC
    #[arg(short = 'E', long, default_value_t = false)]
    only_enabled: bool,

    /// Depth to search
    #[arg(short, long, default_value_t = 3)]
    depth: usize,

    /// Ignore the cached results
    #[arg(long, default_value_t = false)]
    no_cache: bool,

    /// Output JSON
    #[arg(short, long, default_value_t = false)]
    json: bool,
}

type ApkCallsResult = HashMap<DevicePath, Vec<MethodCallPath>>;

fn dump_apk_calls_result(data: &ApkCallsResult, json: bool, show_path: bool) -> anyhow::Result<()> {
    if json {
        serde_json::to_writer(stdout(), data)?;
        return Ok(());
    }

    let printer = Printer::new();

    for (apk, paths) in data {
        printer.println_colored(apk.as_device_str(), color::YELLOW);
        for p in paths {
            if p.is_empty() {
                continue;
            }
            printer.print("  ");
            printer.println_colored(&p.must_get_src_class(), color::CYAN);
            if !show_path {
                continue;
            }
            for call in &p.path {
                printer.print("    ");
                printer.println_colored(call.as_smali(), color::GREY);
            }
        }
    }
    Ok(())
}
