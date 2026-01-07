use std::borrow::Cow;
use std::collections::HashMap;
use std::io::stdout;

use clap::{self, Args, Subcommand};
use dtu::db::sql::device::get_default_devicedb;
use dtu::db::sql::device::models::{Activity, Provider, Receiver, Service};
use sha2::{Digest, Sha256};

use crate::parsers::DevicePathValueParser;
use crate::printer::{color, Printer};
use crate::utils::project_cacheable;
use dtu::db::graph::models::{MethodCallPath, MethodSearch, MethodSearchParams, MethodSpec};
use dtu::db::graph::{get_default_graphdb, GraphDatabase};
use dtu::db::sql::{
    ApkIPC, DeviceDatabase, Enablable, Exportable, MetaDatabase, MetaSqliteDatabase,
};
use dtu::prereqs::Prereq;
use dtu::utils::{hex, ClassName, DevicePath};
use dtu::{Context, DefaultContext};

#[derive(Args)]
pub struct Canned {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find classes that call the given method
    FindCallers(FindCallers),

    /// Find Activitys that call `getIntent`
    FindIntentActivities(FindIntentActivities),

    /// Find IPC that calls Intent.parseUri
    FindParseUri(ParseUri),

    /// Find calls leaving IPC to the given method
    FindIPCCalls(FindIPCCalls),
}

impl Canned {
    pub fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::GraphDatabaseSetup)?;

        let db = get_default_graphdb(&ctx)?;

        match self.command {
            Command::FindCallers(c) => c.run(&db),
            Command::FindIntentActivities(c) => c.run(&ctx, &db),
            Command::FindParseUri(c) => {
                let generic = ApkIPCCallsGeneric::from(c);
                generic.run(&ctx, &db)
            }
            Command::FindIPCCalls(c) => {
                let generic = ApkIPCCallsGeneric::from(c);
                generic.run(&ctx, &db)
            }
        }
    }
}

/// Generic used to search for any ApkIPC call
struct ApkIPCCallsGeneric {
    apk: Option<DevicePath>,
    no_cache: bool,
    json: bool,
    depth: usize,
    cache: Cow<'static, str>,

    method: Cow<'static, str>,
    signature: Cow<'static, str>,
    class: Option<ClassName>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SourcedClass<'a> {
    class: ClassName,
    source: Cow<'a, str>,
}

fn get_search_params<'a>(
    name: Option<&'a str>,
    class: Option<&'a ClassName>,
    signature: Option<&'a str>,
) -> anyhow::Result<MethodSearchParams<'a>> {
    Ok(MethodSearchParams::new(name, class, signature)
        .map_err(|it| anyhow::Error::msg(format!("failed to get search params: {it}")))?)
}

fn get_method_search<'a>(
    name: Option<&'a str>,
    class: Option<&'a ClassName>,
    signature: Option<&'a str>,
    source: Option<&'a str>,
) -> anyhow::Result<MethodSearch<'a>> {
    Ok(MethodSearch::new(
        get_search_params(name, class, signature)?,
        source,
    ))
}

impl ApkIPCCallsGeneric {
    fn run_inner_source(
        &self,
        graphdb: &dyn GraphDatabase,
        source: Option<&str>,
        apk_map: &HashMap<i32, DevicePath>,
        receivers: &[Receiver],
        activities: &[Activity],
        services: &[Service],
        providers: &[Provider],
        mut results: &mut ApkCallsResult,
    ) -> anyhow::Result<()> {
        let class = self.class.as_ref();
        let name = self.method.as_ref();
        let signature = self.signature.as_ref();
        let search = get_method_search(Some(name), class, Some(signature), source)?;

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

        self.run_for_ipc(&mut lookup, &apk_map, &receivers, &mut results)?;
        self.run_for_ipc(&mut lookup, &apk_map, &activities, &mut results)?;
        self.run_for_ipc(&mut lookup, &apk_map, &services, &mut results)?;
        self.run_for_ipc(&mut lookup, &apk_map, &providers, &mut results)?;

        Ok(())
    }

    fn run_inner_parts(
        &self,
        graphdb: &dyn GraphDatabase,
        apk_map: &HashMap<i32, DevicePath>,
        receivers: &[Receiver],
        activities: &[Activity],
        services: &[Service],
        providers: &[Provider],
    ) -> anyhow::Result<ApkCallsResult> {
        let mut results = ApkCallsResult::new();
        let sources = graphdb.get_all_sources()?;

        log::info!(
            "No source specified for depth > 1 search, looking up in parts, {} sources found",
            sources.len()
        );

        for source in sources {
            self.run_inner_source(
                graphdb,
                Some(source.as_str()),
                apk_map,
                receivers,
                activities,
                services,
                providers,
                &mut results,
            )?;
        }
        Ok(results)
    }

    fn run_inner(
        &self,
        ctx: &dyn Context,
        graphdb: &dyn GraphDatabase,
    ) -> anyhow::Result<ApkCallsResult> {
        let db = get_default_devicedb(ctx)?;
        let source = self.apk.as_ref().map(|it| it.as_squashed_str());
        let mut apk_map = HashMap::new();
        for apk in db.get_apks()?.into_iter() {
            apk_map.insert(apk.id, apk.device_path);
        }

        let receivers = db
            .get_receivers()?
            .into_iter()
            .filter(|it| it.is_enabled() && it.is_exported())
            .collect::<Vec<Receiver>>();

        let activities = db
            .get_activities()?
            .into_iter()
            .filter(|it| it.is_enabled() && it.is_exported())
            .collect::<Vec<Activity>>();

        let services = db
            .get_services()?
            .into_iter()
            .filter(|it| it.is_enabled() && it.is_exported())
            .collect::<Vec<Service>>();

        let providers = db
            .get_providers()?
            .into_iter()
            .filter(|it| it.is_enabled() && it.is_exported())
            .collect::<Vec<Provider>>();

        // If there is no source and depth is greater than 1, we should break things up
        // otherwise a ton of memory will be consumed by cozo
        let results = if source.is_none() && self.depth > 1 {
            self.run_inner_parts(
                graphdb,
                &apk_map,
                &receivers,
                &activities,
                &services,
                &providers,
            )?
        } else {
            let mut results = ApkCallsResult::new();

            self.run_inner_source(
                graphdb,
                source,
                &apk_map,
                &receivers,
                &activities,
                &services,
                &providers,
                &mut results,
            )?;
            results
        };

        Ok(results)
    }

    fn run_for_ipc<T: ApkIPC>(
        &self,
        callers: &mut HashMap<SourcedClass, Vec<MethodSpec>>,
        apk_map: &HashMap<i32, DevicePath>,
        ipcs: &[T],
        results: &mut ApkCallsResult,
    ) -> anyhow::Result<()> {
        for ipc in ipcs {
            let source = apk_map
                .get(&ipc.get_apk_id())
                .map(|it| it.as_squashed_str())
                .unwrap_or("framework");

            let name = ipc.get_class_name();

            let search = SourcedClass {
                class: name.clone(),
                source: Cow::Borrowed(source),
            };

            if let Some(path) = callers.get(&search) {
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
        Ok(())
    }

    fn run(&self, ctx: &dyn Context, graphdb: &dyn GraphDatabase) -> anyhow::Result<()> {
        let create = || self.run_inner(ctx, graphdb);

        let cache_file_name = match &self.apk {
            None => format!("all-{}-{}", self.cache, self.depth),
            Some(v) => format!(
                "{}-{}-{}",
                v.as_squashed_str_no_ext(),
                self.cache,
                self.depth
            ),
        };

        let res = project_cacheable(ctx, &cache_file_name, self.no_cache, create)?;
        dump_apk_calls_result(&res, self.json, self.depth > 1)
    }
}

impl From<FindIPCCalls> for ApkIPCCallsGeneric {
    fn from(value: FindIPCCalls) -> Self {
        let mut hasher = Sha256::new();

        hasher.update(value.name.as_bytes());
        hasher.update(value.signature.as_bytes());
        if let Some(v) = &value.class {
            hasher.update(v.as_str().as_bytes());
        }

        let res = hasher.finalize();
        let hex = hex::bytes_to_hex(&res);
        let cache = format!("ipc-calls-{}", hex);

        Self {
            apk: value.apk,
            no_cache: value.no_cache,
            json: value.json,
            depth: value.depth,
            cache: Cow::Owned(cache),
            method: Cow::Owned(String::from(value.name)),
            class: value.class,
            signature: Cow::Owned(String::from(value.signature)),
        }
    }
}

impl From<ParseUri> for ApkIPCCallsGeneric {
    fn from(value: ParseUri) -> Self {
        Self {
            apk: value.apk,
            no_cache: value.no_cache,
            json: value.json,
            depth: 1,
            cache: Cow::Borrowed("ipc-ParseUri"),
            method: Cow::Borrowed("parseUri"),
            class: Some(ClassName::new("Landroid/content/Intent;".into())),
            signature: Cow::Borrowed("Ljava/lang/String;I"),
        }
    }
}

#[derive(Args)]
struct ParseUri {
    /// The APK to search, otherwise all APKs and the framework are searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Ignore the cached results
    #[arg(short, long, default_value_t = false)]
    no_cache: bool,

    /// Output JSON
    #[arg(short, long, default_value_t = false)]
    json: bool,
}

#[derive(Args)]
struct FindIntentActivities {
    /// The APK to search, otherwise all APKs and the framework are searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Ignore the cached results
    #[arg(short, long, default_value_t = false)]
    no_cache: bool,

    /// Output JSON
    #[arg(short, long, default_value_t = false)]
    json: bool,

    /// Only show from privileged applications
    #[arg(short, long, default_value_t = false)]
    only_priv: bool,

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
        let db = get_default_devicedb(ctx)?;
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

    fn run(&self, ctx: &dyn Context, graphdb: &dyn GraphDatabase) -> anyhow::Result<()> {
        let create = || self.run_inner(ctx, graphdb);

        let mut cache_file_name = match &self.apk {
            None => Cow::Borrowed("all-intent-activities"),
            Some(v) => Cow::Owned(format!("{}-intent-activities", v.as_squashed_str_no_ext())),
        };

        if self.strict {
            cache_file_name = Cow::Owned(format!("{}-strict", cache_file_name));
        }

        if self.only_priv {
            cache_file_name = Cow::Owned(format!("{}-only-priv", cache_file_name));
        }

        let res = project_cacheable(ctx, &cache_file_name, self.no_cache, create)?;
        dump_apk_calls_result(&res, self.json, false)
    }
}

#[derive(Args)]
struct FindIPCCalls {
    /// The APK to search, otherwise all APKs are searched
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// Method name
    #[arg(short, long)]
    name: String,

    /// Method signature
    #[arg(short, long)]
    signature: String,

    /// Method class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Depth to search
    #[arg(short, long, default_value_t = 3)]
    depth: usize,

    /// Ignore the cached results
    #[arg(short, long, default_value_t = false)]
    no_cache: bool,

    /// Output JSON
    #[arg(short, long, default_value_t = false)]
    json: bool,
}

#[derive(Args)]
struct FindCallers {
    /// The source of the method to find callers to
    #[arg(short, long)]
    method_source: Option<String>,

    /// The source of the calls
    #[arg(short, long)]
    call_source: Option<String>,

    /// Method name
    #[arg(short, long)]
    name: String,

    /// Method signature
    #[arg(short, long)]
    signature: Option<String>,

    /// Method class
    #[arg(short, long)]
    class: Option<ClassName>,

    /// Depth to search
    #[arg(short, long, default_value_t = 3)]
    depth: usize,
}

impl FindCallers {
    fn run(&self, db: &dyn GraphDatabase) -> anyhow::Result<()> {
        let class_ref = self.class.as_ref();
        let search = get_method_search(
            Some(self.name.as_str()),
            class_ref,
            self.signature.as_ref().map(String::as_str),
            self.method_source.as_ref().map(String::as_str),
        )?;
        let mpaths = db.find_callers(
            &search,
            self.call_source.as_ref().map(String::as_str),
            self.depth,
        )?;

        let printer = Printer::new();

        for p in mpaths {
            let mut iter = p.path.iter().take(p.path.len() - 1);
            let first = match iter.next() {
                Some(v) => v,
                None => continue,
            };

            printer.println_colored(first.as_smali(), color::YELLOW);

            for c in iter {
                printer.print("   ");
                printer.println_colored(c.as_smali(), color::GREY);
            }
        }
        Ok(())
    }
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
