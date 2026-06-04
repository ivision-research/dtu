use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Display;
use std::io;

use clap::{self, Args, Subcommand};

use dtu::db::device::models::{self, Activity, Apk, DiffSource, Provider, Receiver, Service};
use dtu::db::meta::get_default_metadb;
use dtu::db::{ApkIPC, DeviceDatabase, Diffable, PermissionMode, PermissionProtected};
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

mod classes;
use classes::{Children, InterfaceImpl, Parents};

#[derive(Args)]
pub struct List {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct CommonParams {
    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
    only_new: bool,

    /// Set the diff source (only valid with -n/--only-new) otherwise the emulator is the default
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
    fn get_diff_id(&self, ctx: &DefaultContext, db: &DeviceDatabase) -> anyhow::Result<i32> {
        let meta = get_default_metadb(ctx)?;
        Ok(get_diff_source(ctx, &meta, db, &self.diff_source)?.id)
    }

    fn filter_allow<T, U>(&self, val: &T) -> bool
    where
        T: AsRef<U> + Diffable + ?Sized,
        U: ApkIPC,
    {
        if self.only_new && val.in_diff() {
            return false;
        }
        let ipc = val.as_ref();
        if self.only_public && !ipc.is_exported() {
            return false;
        }

        if self.only_enabled && !ipc.is_enabled() {
            return false;
        }

        true
    }

    fn do_list_json<F, R, M, MetaData>(
        &self,
        ctx: &DefaultContext,
        db: &DeviceDatabase,
        func: F,
        meta_func: Option<&M>,
    ) -> anyhow::Result<()>
    where
        MetaData: serde::Serialize,
        M: Fn(&R) -> MetaData + ?Sized,
        R: ApkIPC + Display,
        F: FnOnce(&Self, &DefaultContext, &DeviceDatabase) -> anyhow::Result<Vec<R>>,
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

        let res = func(&self, ctx, db)?
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

                let permission = it.get_generic_permission();

                Some(JsonOutput {
                    id: it.get_id(),
                    class,
                    package,
                    enabled: it.is_enabled(),
                    exported: it.is_exported(),
                    permission: permission.map(String::from),
                    apk: &apk.name,
                    meta,
                    source,
                })
            })
            .collect::<Vec<JsonOutput<'_, MetaData>>>();

        serde_json::to_writer(io::stdout(), &res)?;

        Ok(())
    }

    fn do_list<F, R, M, MetaData>(&self, func: F, meta_func: Option<&M>) -> anyhow::Result<()>
    where
        M: Fn(&R) -> MetaData + ?Sized,
        MetaData: serde::Serialize,
        R: ApkIPC + Display,
        F: FnOnce(&Self, &DefaultContext, &DeviceDatabase) -> anyhow::Result<Vec<R>>,
    {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;

        let db = DeviceDatabase::new(&ctx)?;

        if self.json {
            return self.do_list_json(&ctx, &db, func, meta_func);
        }
        for it in func(&self, &ctx, &db)? {
            println!("{}", it);
        }
        Ok(())
    }

    fn list_receivers(self) -> anyhow::Result<()> {
        self.do_list(
            |p, ctx, db| {
                Ok(if p.only_new {
                    db.get_receiver_diffs_by_diff_id(p.get_diff_id(ctx, db)?)?
                        .into_iter()
                        .filter(|it| p.filter_allow(it))
                        .map(|it| it.receiver)
                        .collect::<Vec<Receiver>>()
                } else {
                    db.get_receivers()?
                })
            },
            None::<&dyn for<'a> Fn(&'a Receiver) -> String>,
        )
    }

    fn list_activities(self) -> anyhow::Result<()> {
        self.do_list(
            |p, ctx, db| {
                Ok(if p.only_new {
                    db.get_activity_diffs_by_diff_id(p.get_diff_id(ctx, db)?)?
                        .into_iter()
                        .filter(|it| p.filter_allow(it))
                        .map(|it| it.activity)
                        .collect::<Vec<Activity>>()
                } else {
                    db.get_activities()?
                })
            },
            None::<&dyn for<'a> Fn(&'a Activity) -> String>,
        )
    }

    fn list_providers(self) -> anyhow::Result<()> {
        self.do_list(
            |p, ctx, db| {
                Ok(if p.only_new {
                    db.get_provider_diffs_by_diff_id(p.get_diff_id(ctx, db)?)?
                        .into_iter()
                        .filter(|it| p.filter_allow(it))
                        .map(|it| it.provider)
                        .collect::<Vec<Provider>>()
                } else {
                    db.get_providers()?
                })
            },
            Some(&|prov: &Provider| {
                #[derive(serde::Serialize)]
                struct MetaData {
                    authorities: Vec<String>,
                    permission: Option<String>,
                    read_permission: Option<String>,
                    write_permission: Option<String>,
                }

                let authorities = prov
                    .get_authorities()
                    .map(String::from)
                    .collect::<Vec<String>>();
                let permission = prov.get_generic_permission().map(String::from);
                let read_permission = prov
                    .get_permission_for_mode(PermissionMode::Read)
                    .map(String::from);
                let write_permission = prov
                    .get_permission_for_mode(PermissionMode::Write)
                    .map(String::from);

                MetaData {
                    authorities,
                    permission,
                    read_permission,
                    write_permission,
                }
            }),
        )
    }
}

#[derive(Args)]
struct ServiceParams {
    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
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

    /// Find parent classes
    #[command()]
    Parents(Parents),
}

impl List {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::Apks(c) => c.run(),
            Command::SystemServices(c) => c.run(),
            Command::SystemServiceMethods(c) => c.run(),
            Command::Providers(p) => p.list_providers(),
            Command::Receivers(p) => p.list_receivers(),
            Command::Activities(p) => p.list_activities(),
            Command::Services(p) => p.list_services(),
            Command::Permissions => self.list_permissions(),
            Command::InterfaceImpl(c) => c.run(),
            Command::Children(c) => c.run(),
            Command::Parents(c) => c.run(),
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
}

impl ServiceParams {
    fn list_services(self) -> anyhow::Result<()> {
        let only_returns_binder = self.only_returns_binder;
        let c = CommonParams {
            only_public: self.only_public,
            only_enabled: self.only_enabled,
            json: self.json,
            only_new: self.only_new,
            diff_source: self.diff_source.clone(),
        };
        c.do_list(
            |p, ctx, db| {
                let services = if p.only_new {
                    db.get_service_diffs_by_diff_id(c.get_diff_id(ctx, db)?)?
                        .into_iter()
                        .filter(|it| c.filter_allow(it))
                        .map(|it| it.service)
                        .collect::<Vec<Service>>()
                } else {
                    db.get_services()?
                };

                if !only_returns_binder {
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
}
