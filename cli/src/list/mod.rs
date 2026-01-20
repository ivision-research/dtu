use std::fmt::Display;

use clap::{self, Args, Subcommand};

use dtu::db::device::models;
use dtu::db::{ApkIPC, DeviceDatabase, DeviceSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::ensure_prereq;
use dtu::DefaultContext;

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
    /// Only show public entries
    #[arg(short = 'P', long)]
    only_public: bool,

    /// Only show enabled entries
    #[arg(short = 'E', long)]
    only_enabled: bool,
}

#[derive(Args)]
struct ServiceParams {
    /// Only show public services
    #[arg(short = 'P', long)]
    only_public: bool,

    /// Only show enabled services
    #[arg(short = 'E', long)]
    only_enabled: bool,

    /// Only show Services that return a binder
    #[arg(short = 'B', long)]
    only_returns_binder: bool,
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
        let db = DeviceSqliteDatabase::new(&ctx)?;
        let perms = db.get_permissions()?;
        for p in &perms {
            println!("{}", p);
        }
        Ok(())
    }

    fn do_list<F, R>(&self, p: &CommonParams, func: F) -> anyhow::Result<()>
    where
        R: ApkIPC + Display,
        F: FnOnce(&dyn DeviceDatabase) -> anyhow::Result<Vec<R>>,
    {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::SQLDatabaseSetup)?;
        let db = DeviceSqliteDatabase::new(&ctx)?;
        let unfiltered = func(&db)?;
        let items = unfiltered.iter().filter(|it| {
            if p.only_public && !it.is_exported() {
                return false;
            }
            if p.only_enabled && !it.is_enabled() {
                return false;
            }
            true
        });
        for it in items {
            println!("{}", it);
        }
        Ok(())
    }

    fn list_receivers(&self, p: &CommonParams) -> anyhow::Result<()> {
        self.do_list(p, |db| Ok(db.get_receivers()?))
    }

    fn list_services(&self, p: &ServiceParams) -> anyhow::Result<()> {
        let c = CommonParams {
            only_public: p.only_public,
            only_enabled: p.only_enabled,
        };
        self.do_list(&c, |db| {
            let services = db.get_services()?;
            if !p.only_returns_binder {
                return Ok(services);
            }
            let filtered = services
                .into_iter()
                .filter(|it| it.returns_binder.is_true_or_unknown())
                .collect::<Vec<models::Service>>();
            Ok(filtered)
        })
    }

    fn list_activities(&self, p: &CommonParams) -> anyhow::Result<()> {
        self.do_list(p, |db| Ok(db.get_activities()?))
    }

    fn list_providers(&self, p: &CommonParams) -> anyhow::Result<()> {
        self.do_list(p, |db| Ok(db.get_providers()?))
    }
}
