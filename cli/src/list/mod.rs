use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;
use std::io;

use clap::{self, Args, Subcommand};

use dtu::db::device::models::{self, Activity, Apk, DiffSource, Provider, Receiver, Service};
use dtu::db::meta::get_default_metadb;
use dtu::db::{ApkIPC, DeviceDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::{ensure_prereq, ClassName};
use dtu::DefaultContext;

use crate::diff::get_diff_source;
use crate::parsers::DiffSourceValueParser;

mod apks;
use apks::Apks;

mod system_services;
use system_services::SystemServices;

mod system_service_methods;
use system_service_methods::SystemServiceMethods;

mod interface_impl;
use interface_impl::InterfaceImpl;

mod children;
use children::Children;

#[derive(Args)]
pub struct List {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct CommonParams {
    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'N', long)]
    only_new: bool,

    /// Set the diff source (only valid with -N/--only-new) otherwise the emulator is the default
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,

    /// Only show public entries
    #[arg(short = 'P', long)]
    only_public: bool,

    /// Only show enabled entries
    #[arg(short = 'E', long)]
    only_enabled: bool,

    #[arg(short = 'j', long = "json")]
    json: bool,
}

impl CommonParams {
    fn get_diff_id(&self, ctx: &DefaultContext) -> anyhow::Result<i32> {
        let meta = get_default_metadb(ctx)?;
        Ok(get_diff_source(ctx, &meta, &self.diff_source)?.id)
    }
}

#[derive(Args)]
struct ServiceParams {
    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'N', long)]
    only_new: bool,

    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,

    /// Only show public services
    #[arg(short = 'P', long)]
    only_public: bool,

    /// Only show enabled services
    #[arg(short = 'E', long)]
    only_enabled: bool,

    /// Only show Services that return a binder
    #[arg(short = 'B', long)]
    only_returns_binder: bool,

    #[arg(short = 'j', long = "json")]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Show all APKs
    #[command()]
    Apks(Apks),

    /// Show all system services
    #[command()]
    SystemServices(SystemServices),

    /// Show all system service methods
    #[command()]
    SystemServiceMethods(SystemServiceMethods),

    /// List all known ContentProviders
    #[command()]
    Providers(CommonParams),

    /// List all known BroadcastReceivers
    #[command()]
    Receivers(CommonParams),

    /// List all known Activities
    #[command()]
    Activities(CommonParams),

    /// List all known Services
    #[command()]
    Services(ServiceParams),

    /// List all known Permissions
    #[command()]
    Permissions,

    /// Find interface implementations
    #[command()]
    InterfaceImpl(InterfaceImpl),

    /// Find child classes
    #[command()]
    Children(Children),
}

impl List {
    pub fn run(&self) -> anyhow::Result<()> {
        match &self.command {
            Command::Apks(c) => c.run(),
            Command::SystemServices(c) => c.run(),
            Command::SystemServiceMethods(c) => c.run(),
            Command::Providers(p) => self.list_providers(p),
            Command::Receivers(p) => self.list_receivers(p),
            Command::Activities(p) => self.list_activities(p),
            Command::Services(p) => self.list_services(p),
            Command::Permissions => self.list_permissions(),
            Command::InterfaceImpl(c) => c.run(),
            Command::Children(c) => c.run(),
        }
    }

    fn list_permissions(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;
        let db = DeviceDatabase::new(&ctx)?;
        let perms = db.get_permissions()?;
        for p in &perms {
            println!("{}", p);
        }
        Ok(())
    }

    fn get_items<F, R>(
        &self,
        ctx: &DefaultContext,
        db: &DeviceDatabase,
        p: &CommonParams,
        func: F,
    ) -> anyhow::Result<Vec<R>>
    where
        R: ApkIPC + Display,
        F: FnOnce(&DefaultContext, &DeviceDatabase) -> anyhow::Result<Vec<R>>,
    {
        let unfiltered = func(ctx, db)?;

        Ok(unfiltered
            .into_iter()
            .filter(|it| {
                if p.only_public && !it.is_exported() {
                    return false;
                }
                if p.only_enabled && !it.is_enabled() {
                    return false;
                }
                true
            })
            .collect())
    }

    fn do_list_json<F, R, M, MetaData>(
        &self,
        ctx: &DefaultContext,
        db: &DeviceDatabase,
        p: &CommonParams,
        func: F,
        meta_func: Option<&M>,
    ) -> anyhow::Result<()>
    where
        MetaData: serde::Serialize,
        M: Fn(&R) -> MetaData + ?Sized,
        R: ApkIPC + Display,
        F: FnOnce(&DefaultContext, &DeviceDatabase) -> anyhow::Result<Vec<R>>,
    {
        let apks: HashMap<i32, Apk> =
            HashMap::from_iter(db.get_apks()?.into_iter().map(|apk| (apk.id, apk)));

        #[derive(serde::Serialize)]
        struct JsonOutput<'a, MD: serde::Serialize> {
            id: i32,
            class: ClassName,
            package: Cow<'a, str>,
            enabled: bool,
            exported: bool,
            permission: Option<String>,
            apk: &'a str,
            source: &'a str,
            meta: Option<MD>,
        }

        let res = self
            .get_items(ctx, db, p, func)?
            .into_iter()
            .filter_map(|it| {
                let apk = apks.get(&it.get_apk_id())?;

                let source = if apk.app_name == "android" {
                    "framework"
                } else {
                    apk.device_path.as_squashed_str()
                };

                let meta = if let Some(f) = &meta_func {
                    Some(f(&it))
                } else {
                    None
                };

                let mut class = it.get_class_name();
                let mut package: Cow<'_, str> = Cow::Owned(it.get_package().to_string());

                if package.is_empty() {
                    if class.has_pkg() {
                        package = Cow::Owned(class.pkg_as_java().to_string());
                    } else {
                        package = Cow::Borrowed(apk.app_name.as_str());
                    }
                }

                if !class.has_pkg() {
                    class = class.with_new_package(&package);
                }

                Some(JsonOutput {
                    id: it.get_id(),
                    class,
                    package,
                    enabled: it.is_enabled(),
                    exported: it.is_exported(),
                    permission: it.get_generic_permission().map(String::from),
                    apk: &apk.name,
                    meta,
                    source,
                })
            })
            .collect::<Vec<JsonOutput<'_, MetaData>>>();

        serde_json::to_writer(io::stdout(), &res)?;

        Ok(())
    }

    fn do_list<F, R, M, MetaData>(
        &self,
        p: &CommonParams,
        func: F,
        meta_func: Option<&M>,
    ) -> anyhow::Result<()>
    where
        M: Fn(&R) -> MetaData + ?Sized,
        MetaData: serde::Serialize,
        R: ApkIPC + Display,
        F: FnOnce(&DefaultContext, &DeviceDatabase) -> anyhow::Result<Vec<R>>,
    {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;

        let db = DeviceDatabase::new(&ctx)?;

        if p.json {
            return self.do_list_json(&ctx, &db, p, func, meta_func);
        }
        for it in self.get_items(&ctx, &db, p, func)? {
            println!("{}", it);
        }
        Ok(())
    }

    fn list_receivers(&self, p: &CommonParams) -> anyhow::Result<()> {
        self.do_list(
            p,
            |ctx, db| {
                Ok(if p.only_new {
                    db.get_receiver_diffs_by_diff_id(p.get_diff_id(ctx)?)?
                        .into_iter()
                        .map(|it| it.receiver)
                        .collect::<Vec<Receiver>>()
                } else {
                    db.get_receivers()?
                })
            },
            None::<&dyn for<'a> Fn(&'a Receiver) -> String>,
        )
    }

    fn list_services(&self, p: &ServiceParams) -> anyhow::Result<()> {
        let c = CommonParams {
            only_public: p.only_public,
            only_enabled: p.only_enabled,
            json: p.json,
            only_new: p.only_new,
            diff_source: p.diff_source.clone(),
        };
        self.do_list(
            &c,
            |ctx, db| {
                let services = if p.only_new {
                    db.get_service_diffs_by_diff_id(c.get_diff_id(ctx)?)?
                        .into_iter()
                        .map(|it| it.service)
                        .collect::<Vec<Service>>()
                } else {
                    db.get_services()?
                };

                if !p.only_returns_binder {
                    return Ok(services);
                }
                let filtered = services
                    .into_iter()
                    .filter(|it| it.returns_binder.is_true_or_unknown())
                    .collect::<Vec<models::Service>>();
                Ok(filtered)
            },
            None::<&dyn for<'a> Fn(&'a Service) -> String>,
        )
    }

    fn list_activities(&self, p: &CommonParams) -> anyhow::Result<()> {
        self.do_list(
            p,
            |ctx, db| {
                Ok(if p.only_new {
                    db.get_activity_diffs_by_diff_id(p.get_diff_id(ctx)?)?
                        .into_iter()
                        .map(|it| it.activity)
                        .collect::<Vec<Activity>>()
                } else {
                    db.get_activities()?
                })
            },
            None::<&dyn for<'a> Fn(&'a Activity) -> String>,
        )
    }

    fn list_providers(&self, p: &CommonParams) -> anyhow::Result<()> {
        self.do_list(
            p,
            |ctx, db| {
                Ok(if p.only_new {
                    db.get_provider_diffs_by_diff_id(p.get_diff_id(ctx)?)?
                        .into_iter()
                        .map(|it| it.provider)
                        .collect::<Vec<Provider>>()
                } else {
                    db.get_providers()?
                })
            },
            Some(&|prov: &Provider| {
                prov.get_authorities()
                    .map(String::from)
                    .collect::<Vec<String>>()
            }),
        )
    }
}
