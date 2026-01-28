use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::DirEntry;
use std::ops::Deref;
use std::path::PathBuf;

use base64::Engine;
use diesel::{insert_into, prelude::*, update};
use regex::Regex;
use sha2::{Digest, Sha256};
use smalisa::instructions::{InvArgs, Invocation};
use smalisa::{
    parse_class, FieldRef, Lexer, Line, LineParse, Literal, Method, MethodHeader, MethodLine,
    MethodRef, Parser, Type,
};

use dtu_proc_macro::{define_setters, wraps_base_error};

use crate::adb::{Adb, ExecAdb, ADB_CONFIG_KEY};
use crate::command::err_on_status;
use crate::context::Context;
use crate::db::common::Error;
use crate::db::device::db::{DeviceDatabase, SqlConnection};
use crate::db::device::models::*;
use crate::db::device::schema::*;
use crate::db::graph::models::{ClassSearch, ClassSpec};
use crate::db::graph::{GraphDatabase, FRAMEWORK_SOURCE};
use crate::db::MetaDatabase;
use crate::fsdump::FSDumpAccess;
use crate::manifest::{self, ApktoolManifestResolver, IPC};
use crate::prereqs::Prereq;
use crate::tasks::task::{EventMonitor, TaskCancelCheck};
use crate::unknownbool::UnknownBool;
use crate::utils::class_name::ClassName;
use crate::utils::device_path::DevicePath;
use crate::utils::fs::{
    find_files_for_class, find_smali_file_for_class, path_has_ext, path_must_str,
};
use crate::utils::open_file;
use crate::Manifest;

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("pull and decompile required for setup")]
    NoPullAndDecompile,
    #[error("no graph database")]
    NoGraphDatabase,
    #[error("already setup")]
    AlreadySetup,
    #[error("database error {0}")]
    DB(Error),
    #[error("no frameworks dir")]
    NoFrameworksDir,
    #[error("user cancelled")]
    Cancelled,
    #[error("smalisa error: {0}")]
    Smalisa(String),
    #[error("failed to get s3 asset: {0}")]
    S3AssetFailure(String),
    #[error("{0}")]
    Generic(String),
    #[error("invalid manifest for {0} - {1}")]
    InvalidManifest(String, String),
}

impl From<diesel::result::Error> for SetupError {
    fn from(value: diesel::result::Error) -> Self {
        Self::DB(value.into())
    }
}

impl<'a> From<smalisa::ParseError<'a>> for SetupError {
    fn from(value: smalisa::ParseError) -> Self {
        Self::Smalisa(value.to_string())
    }
}

impl<'a> From<smalisa::LexError<'a>> for SetupError {
    fn from(value: smalisa::LexError) -> Self {
        Self::Smalisa(value.to_string())
    }
}

impl From<Error> for SetupError {
    fn from(value: Error) -> Self {
        Self::DB(value)
    }
}

pub type SetupResult<T> = Result<T, SetupError>;

#[define_setters]
pub struct SetupOptions {
    pub force: bool,
}

impl Default for SetupOptions {
    fn default() -> Self {
        Self { force: false }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct ApkIdentifier {
    pub apk_id: usize,
    pub dir_id: usize,
}

impl ApkIdentifier {
    pub fn new(apk_id: usize, dir_id: usize) -> Self {
        Self { apk_id, dir_id }
    }
}

/// Events sent out during database setup
pub enum SetupEvent {
    DiscoveredServices {
        entries: Vec<ServiceMeta>,
    },

    StartAddingSystemService {
        service: String,
        service_id: usize,
        interface: Option<ClassName>,
    },

    FoundSystemServiceImpl {
        service_id: usize,
        implementation: ClassName,
        source: String,
    },

    FoundSystemServiceMethod {
        service_id: usize,
        name: String,
        txn_id: i32,
        signature: Option<String>,
        return_type: Option<String>,
    },

    DoneAddingSystemService {
        service_id: usize,
    },

    StartedApksForDir {
        dir: PathBuf,
        /// This is just an opaque value to help keep track of the given
        /// directory. It has no meaning outside of each run.
        dir_id: usize,
        count: usize,
    },

    DoneApksForDir {
        dir_id: usize,
        success: bool,
    },

    StartAddingApk {
        path: DevicePath,
        /// This is just an opaque value to help keep track of the given APK.
        /// It has no meaning outside of each run.
        identifier: ApkIdentifier,
    },

    DoneAddingApk {
        identifier: ApkIdentifier,
        success: bool,
    },

    AddingApkPermission {
        identifier: ApkIdentifier,
        permission: String,
    },

    AddingApkProvider {
        identifier: ApkIdentifier,
        class_name: ClassName,
    },

    AddingApkActivity {
        identifier: ApkIdentifier,
        class_name: ClassName,
    },

    AddingApkService {
        identifier: ApkIdentifier,
        class_name: ClassName,
    },

    AddingApkReceiver {
        identifier: ApkIdentifier,
        class_name: ClassName,
    },
}

pub type PackageCallback<'a> = dyn FnMut(&str) -> anyhow::Result<()> + 'a;

/// Provides access to device resources for the database setup
pub trait DatabaseSetupHelper {
    fn get_props(&self) -> crate::Result<HashMap<String, String>>;
    fn list_services(&self) -> crate::Result<Vec<ServiceMeta>>;
    fn list_packages(&self, on_pkg: &mut PackageCallback) -> crate::Result<()>;
}

macro_rules! on_event {
    ($mon:expr, $event:expr) => {
        if let Some(m) = $mon {
            m.on_event($event);
        }
    };
}

struct AddManifestTask<'a> {
    apk_id: i32,
    ctx: &'a dyn Context,
    pkg: Cow<'a, str>,
    device_path: &'a DevicePath,
    cancel: &'a TaskCancelCheck,
    monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
    identifier: ApkIdentifier,
    resolver: &'a ApktoolManifestResolver,
    manifest: &'a Manifest,
}

impl<'a> AddManifestTask<'a> {
    fn run(self, conn: &mut SqlConnection) -> SetupResult<()> {
        log::trace!("Adding apk receivers");
        self.add_manifest_receivers(conn, self.manifest.get_receivers())?;

        self.cancel_check()?;
        log::trace!("Adding apk services");

        self.add_manifest_services(conn, self.manifest.get_services())?;

        self.cancel_check()?;

        log::trace!("Adding apk providers");

        self.add_manifest_providers(conn, self.manifest.get_providers())?;

        self.cancel_check()?;

        log::trace!("Adding apk activities");

        self.add_manifest_activities(
            conn,
            self.manifest.get_activities(),
            self.manifest.get_activity_aliases(),
        )?;

        self.cancel_check()?;

        log::trace!("Adding apk permissions");

        self.add_manifest_permissions(
            conn,
            self.manifest.get_permissions(),
            self.manifest.get_uses_permissions(),
        )?;

        log::trace!("Adding protected broadcasts");
        self.add_protected_broadcasts(conn, self.manifest.get_protected_broadcasts())?;

        Ok(())
    }

    #[inline]
    fn cancel_check(&self) -> SetupResult<()> {
        self.cancel.check(SetupError::Cancelled)
    }

    fn add_protected_broadcasts(
        &self,
        conn: &mut SqlConnection,
        pbs: &[manifest::ProtectedBroadcast],
    ) -> SetupResult<()> {
        for pb in pbs {
            self.add_protected_broadcast(conn, &pb)?;
        }
        Ok(())
    }

    fn add_protected_broadcast(
        &self,
        conn: &mut SqlConnection,
        pb: &manifest::ProtectedBroadcast,
    ) -> SetupResult<()> {
        let ins = InsertProtectedBroadcast {
            name: &pb.name(self.resolver),
        };
        if let Err(e) = insert_into(protected_broadcasts::table)
            .values(&ins)
            .execute(conn)
            .map_err(Error::from)
        {
            if !matches!(e, Error::UniqueViolation(_)) {
                return Err(e.into());
            }
        }

        Ok(())
    }

    fn add_manifest_receivers(
        &self,
        conn: &mut SqlConnection,
        rcvers: &[manifest::Receiver],
    ) -> SetupResult<()> {
        for r in rcvers {
            self.add_manifest_receiver(conn, &r)?;
        }
        Ok(())
    }

    fn add_manifest_receiver(
        &self,
        conn: &mut SqlConnection,
        rcv: &manifest::Receiver,
    ) -> SetupResult<()> {
        let common = IPCCommon::from_ipc(rcv, self.resolver);

        let enabled = common.is_enabled();
        let exported = common.is_exported();
        let permission = common.permission();
        let (raw_class_name, pkg_name) = common.class_and_package(&self.pkg);
        let class_name = ClassName::from_split_manifest(pkg_name, raw_class_name);

        on_event!(
            &self.monitor,
            SetupEvent::AddingApkReceiver {
                identifier: self.identifier,
                class_name: class_name.clone(),
            }
        );

        let ins = InsertReceiver {
            apk_id: self.apk_id,
            permission,
            class_name,
            exported,
            enabled,
            pkg: pkg_name,
        };
        insert_into(receivers::table).values(&ins).execute(conn)?;
        Ok(())
    }

    fn add_manifest_provider(
        &self,
        conn: &mut SqlConnection,
        prov: &manifest::Provider,
    ) -> SetupResult<()> {
        let common = IPCCommon::from_ipc(prov, self.resolver);

        let (raw_class_name, pkg_name) = common.class_and_package(&self.pkg);
        let class_name = ClassName::from_split_manifest(pkg_name, raw_class_name);

        let write_perm = prov.write_permission(self.resolver);
        let read_perm = prov.read_permission(self.resolver);
        let authorities = prov.authorities(self.resolver);
        let grant_uri_permissions = prov.grant_uri_permissions(self.resolver).unwrap_or(false);
        let exported = common.is_exported();
        let enabled = common.is_enabled();
        let perms = common.permission();

        on_event!(
            &self.monitor,
            SetupEvent::AddingApkProvider {
                identifier: self.identifier,
                class_name,
            }
        );
        let ins = InsertProvider {
            authorities: &authorities,
            grant_uri_permissions,
            exported,
            enabled,
            apk_id: self.apk_id,
            name: &common.name,
            permission: perms,
            read_permission: read_perm.as_ref().map(|it| it.as_ref()),
            write_permission: write_perm.as_ref().map(|it| it.as_ref()),
        };
        insert_into(providers::table).values(&ins).execute(conn)?;

        Ok(())
    }

    fn add_manifest_providers(
        &self,
        conn: &mut SqlConnection,
        providers: &[manifest::Provider],
    ) -> SetupResult<()> {
        for p in providers {
            self.add_manifest_provider(conn, &p)?;
        }
        Ok(())
    }

    fn add_manifest_activities(
        &self,
        conn: &mut SqlConnection,
        activities: &[manifest::Activity],
        aliases: &[manifest::Activity],
    ) -> SetupResult<()> {
        for act in activities {
            self.add_manifest_activity(conn, act)?;
        }

        for act in aliases {
            self.add_manifest_activity(conn, act)?;
        }

        Ok(())
    }

    fn add_manifest_activity(
        &self,
        conn: &mut SqlConnection,
        act: &manifest::Activity,
    ) -> SetupResult<()> {
        let common = IPCCommon::from_ipc(act, self.resolver);
        let (raw_class_name, pkg_name) = common.class_and_package(&self.pkg);
        let class_name = ClassName::from_split_manifest(pkg_name, raw_class_name);
        on_event!(
            &self.monitor,
            SetupEvent::AddingApkActivity {
                identifier: self.identifier,
                class_name: class_name.clone(),
            }
        );
        let ins = InsertActivity {
            class_name,
            exported: common.is_exported(),
            enabled: common.is_enabled(),
            pkg: pkg_name,
            apk_id: self.apk_id,
            permission: common.permission(),
        };
        insert_into(activities::table).values(&ins).execute(conn)?;
        Ok(())
    }

    fn add_manifest_services(
        &self,
        conn: &mut SqlConnection,
        services: &[manifest::Service],
    ) -> SetupResult<()> {
        for s in services {
            self.add_manifest_service(conn, &s)?;
        }
        Ok(())
    }

    fn add_manifest_service(
        &self,
        conn: &mut SqlConnection,
        svc: &manifest::Service,
    ) -> SetupResult<()> {
        let common = IPCCommon::from_ipc(svc, self.resolver);

        let enabled = common.is_enabled();
        let exported = common.is_exported();
        let permission = common.permission();
        let (raw_class_name, pkg_name) = common.class_and_package(&self.pkg);

        let class_name = ClassName::from_split_manifest(pkg_name, raw_class_name);

        let returns_binder = self.service_on_bind_returns_nonnull(&class_name);

        on_event!(
            &self.monitor,
            SetupEvent::AddingApkService {
                identifier: self.identifier,
                class_name: class_name.clone()
            }
        );
        let ins = InsertService {
            class_name,
            exported,
            enabled,
            pkg: pkg_name,
            apk_id: self.apk_id,
            permission,
            returns_binder,
        };
        insert_into(services::table).values(&ins).execute(conn)?;

        Ok(())
    }

    fn add_manifest_permissions(
        &self,
        conn: &mut SqlConnection,
        permissions: &[manifest::Permission],
        uses_permissions: &[manifest::UsesPermission],
    ) -> SetupResult<()> {
        for p in permissions {
            self.add_manifest_permission(conn, p)?;
        }

        for up in uses_permissions {
            let name = up.name(self.resolver);
            if name.is_empty() {
                continue;
            }
            let ins = InsertApkPermission {
                apk_id: self.apk_id,
                name: &name,
            };
            match insert_into(apk_permissions::table)
                .values(&ins)
                .execute(conn)
                .map_err(Error::from)
            {
                Err(Error::UniqueViolation(_)) => {
                    log::warn!(
                        "unique constraint violation for APK permission {}, source: {}",
                        name,
                        self.device_path.as_device_str()
                    );
                }
                Err(e) => return Err(e.into()),
                _ => {}
            }
        }

        Ok(())
    }

    fn service_on_bind_returns_nonnull(&self, class: &ClassName) -> UnknownBool {
        let path = match find_smali_file_for_class(&self.ctx, class, Some(self.device_path)) {
            None => {
                log::warn!("couldn't find smali file for {}", class);
                return UnknownBool::Unknown;
            }
            Some(p) => p,
        };

        let mut file = match open_file(&path) {
            Err(e) => {
                log::error!("couldn't open smali file for {}: {}", class, e);
                return UnknownBool::Unknown;
            }
            Ok(f) => f,
        };

        let lexer = Lexer::new(&mut file);
        let mut parser = Parser::new(lexer);

        let parsed = match parse_class(&mut parser) {
            Err(e) => {
                log::error!("couldn't parse class {}: {}", class, e);
                return UnknownBool::Unknown;
            }
            Ok(c) => c,
        };

        let method = match parsed
            .methods
            .iter()
            .find(|it| it.name == "onBind" && it.args == "Landroid/content/Intent;")
        {
            Some(m) => m,
            None => {
                // Could potentially go into the graph database to find a parent
                // here because onBind _must_ be defined
                log::warn!("class {} doesn't have an onBind method", class);
                return UnknownBool::Unknown;
            }
        };

        let instructions = method
            .lines
            .iter()
            .map(|it| match it {
                MethodLine::Instruction(ins) => Some(ins),
                _ => None,
            })
            .filter(|it| it.is_some())
            .map(|it| it.unwrap())
            .collect::<Vec<&Invocation>>();

        // Returning null is always just 2 lines:
        //
        // const/4 v0, 0x0
        // return-object v0
        //
        // TODO:
        // There are obviously other cases in which we can be returning null
        // but doing some sort of logging or something, but for now we're just
        // trying to get some sort of data

        if instructions.len() != 2 {
            return UnknownBool::True;
        }

        // Continuing with the previous train of thought, if the instruction
        // invocation doesn't take a register and a number then it is returning
        // something nonnull
        match instructions[0].args() {
            // This _can't_ be anything but 0 to be valid Java code, so it's
            // returning null
            InvArgs::OneRegNum(_, _) => UnknownBool::False,
            _ => UnknownBool::True,
        }
    }

    fn add_manifest_permission(
        &self,
        conn: &mut SqlConnection,
        perm: &manifest::Permission,
    ) -> SetupResult<()> {
        let name = perm.name(self.resolver);
        let protection_level = perm.protection_level(self.resolver);

        on_event!(
            &self.monitor,
            SetupEvent::AddingApkPermission {
                identifier: self.identifier,
                permission: name.to_string(),
            }
        );

        let ins = InsertPermission {
            name: &name,
            protection_level: &protection_level,
            source_apk_id: self.apk_id,
        };

        match insert_into(permissions::table)
            .values(&ins)
            .execute(conn)
            .map_err(Error::from)
        {
            Err(Error::UniqueViolation(_)) => {
                log::warn!(
                    "unique constraint violation for permission {}, source: {}",
                    name,
                    self.device_path.as_device_str()
                );
                Ok(())
            }
            Err(e) => Err(e.into()),
            _ => Ok(()),
        }
    }
}

pub struct AddApkTask<'a> {
    ctx: &'a dyn Context,
    conn: &'a mut SqlConnection,
    apktool_out_dir: &'a PathBuf,
    device_path: &'a DevicePath,
    priv_app_paths: Option<&'a HashSet<String>>,
    cancel: &'a TaskCancelCheck,
    monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
    identifier: ApkIdentifier,
}

impl DeviceDatabase {
    pub fn setup(
        &self,
        ctx: &dyn Context,
        opts: SetupOptions,
        helper: &dyn DatabaseSetupHelper,
        monitor: Option<&dyn EventMonitor<SetupEvent>>,
        graph: &dyn GraphDatabase,
        meta: &dyn MetaDatabase,
        cancel: TaskCancelCheck,
    ) -> SetupResult<()> {
        let mut prog = meta.get_progress(Prereq::SQLDatabaseSetup)?;
        if prog.completed {
            if opts.force {
                prog.completed = false;
                log::debug!("wiping database due to force");
                self.wipe()?;
                meta.update_progress(&prog)?;
                meta.update_prereq(Prereq::EmulatorDiff, false)?;
            } else {
                return Err(SetupError::AlreadySetup);
            }
        } else if opts.force {
            // Need to fall through to here just in case it was only partially
            // done.
            log::debug!("wiping database due to force");
            self.wipe()?;
        }

        let task = DBSetupTask::new(ctx, helper, graph, self, monitor, cancel);
        let res = task.run();

        if res.is_ok() {
            prog.completed = true;
            meta.update_progress(&prog)?;
        }

        res
    }
}

pub struct DBSetupTask<'a> {
    ctx: &'a dyn Context,
    helper: &'a dyn DatabaseSetupHelper,
    graph: &'a dyn GraphDatabase,
    db: &'a DeviceDatabase,
    monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
    cancel: TaskCancelCheck,
}

/// Task to add a single system service to the database
///
/// If the stub_path and impl_path are not given, there are no guarantees
/// that the $Stub and implementation will be found.
pub struct AddSystemServiceTask<'a> {
    ctx: &'a dyn Context,
    task_id: usize,
    service: &'a ServiceMeta,
    graph: Option<&'a dyn GraphDatabase>,
    db: &'a DeviceDatabase,
    monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
    cancel: &'a TaskCancelCheck,

    allow_exists: bool,

    stub_path: Option<&'a PathBuf>,
    impl_path: Option<&'a PathBuf>,
    source: Option<&'a str>,
}

impl<'a> AddSystemServiceTask<'a> {
    pub fn new(
        ctx: &'a dyn Context,
        task_id: usize,
        service: &'a ServiceMeta,
        graph: Option<&'a dyn GraphDatabase>,
        db: &'a DeviceDatabase,
        monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
        cancel: &'a TaskCancelCheck,
    ) -> Self {
        Self {
            ctx,
            task_id,
            service,
            monitor,
            graph,
            cancel,
            db,
            allow_exists: false,
            stub_path: None,
            impl_path: None,
            source: None,
        }
    }

    pub fn set_allow_exists(&mut self, allow: bool) -> &mut Self {
        self.allow_exists = allow;
        self
    }

    pub fn set_source(&mut self, source: Option<&'a str>) -> &mut Self {
        self.source = source;
        self
    }

    pub fn set_impl_path(&mut self, path: Option<&'a PathBuf>) -> &mut Self {
        self.impl_path = path;
        self
    }

    pub fn set_stub_path(&mut self, path: Option<&'a PathBuf>) -> &mut Self {
        self.stub_path = path;
        self
    }

    pub fn run(&self) -> SetupResult<()> {
        on_event!(
            &self.monitor,
            SetupEvent::StartAddingSystemService {
                service: self.service.service_name.clone(),
                service_id: self.task_id,
                interface: self.service.iface.clone(),
            }
        );

        let res = self.db.with_transaction(|c| self.add_service(c));
        on_event!(
            &self.monitor,
            SetupEvent::DoneAddingSystemService {
                service_id: self.task_id,
            }
        );
        res
    }

    fn add_service(&self, conn: &mut SqlConnection) -> SetupResult<()> {
        let iface = self.service.iface.as_ref();

        let ins = InsertSystemService::new(&self.service.service_name, UnknownBool::Unknown)
            .set_iface(iface.cloned());

        let id = match insert_into(system_services::table)
            .values(&ins)
            .returning(system_services::id)
            .get_result(conn)
            .map_err(Error::from)
        {
            Err(Error::UniqueViolation(_)) if self.allow_exists => system_services::table
                .filter(system_services::name.eq(&self.service.service_name))
                .select(system_services::id)
                .get_result::<i32>(conn)?,
            Err(e) => return Err(e.into()),
            Ok(v) => v,
        };

        if iface.is_none() {
            return Ok(());
        }
        self.add_service_interface_details(conn, id, iface.unwrap())
    }

    #[inline]
    fn get_service_name(&self) -> &str {
        self.service.service_name.as_str()
    }

    /// Uses the known service interface to add details such as methods and
    /// implementations
    ///
    /// If impl_path isn't set, we use the graph database and the given Context
    /// to attempt to find implementations and their associated smali files
    fn add_service_interface_details(
        &self,
        conn: &mut SqlConnection,
        service_db_id: i32,
        iface: &ClassName,
    ) -> SetupResult<()> {
        let stub = ClassName::from(format!("{}$Stub", iface.get_java_name()));

        match self.add_methods_from_iface_and_stub(conn, service_db_id, iface, &stub) {
            Err(SetupError::Smalisa(err)) => {
                log::error!("smalisa error {} for stub {}", err, stub)
            }
            Err(e) => return Err(e),
            _ => {}
        }

        match self.impl_path {
            None => {
                let imp = match self.find_and_add_impls(conn, service_db_id, &stub)? {
                    None => return Ok(()),
                    Some(it) => it,
                };
                self.update_method_hashes(conn, service_db_id, &imp)?;
            }
            Some(p) => {
                let class_name = self.get_class_from_file(p)?;

                let source = match self.source {
                    Some(v) => v,
                    None => "UNKNOWN",
                };

                let ins = InsertSystemServiceImpl::new(service_db_id, source, class_name);
                insert_into(system_service_impls::table)
                    .values(&ins)
                    .execute(conn)?;
                self.update_method_hashes_from_path(conn, service_db_id, p)?;
            }
        }

        Ok(())
    }

    /// Parse a file with smalisa to find the class name
    fn get_class_from_file(&self, path: &PathBuf) -> SetupResult<ClassName> {
        let mut file = open_file(&path)?;
        let lexer = Lexer::new(&mut file);
        let mut parser = Parser::new(lexer);
        let mut line = Line::default();
        loop {
            match parser.parse_line_into(&mut line) {
                Err(e) if e.is_eof() => {
                    return Err(SetupError::Generic(format!(
                        "invalid smali file {} doesn't contain .class directive",
                        path_must_str(path)
                    )))
                }
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            match line {
                Line::Class(_, name) => return Ok(ClassName::from(name)),
                _ => {}
            }
        }
    }

    /// Use smalisa on the given path to update all method hashes for the
    /// given system service.
    ///
    /// This requires having found (or been supplied with) a valid
    /// implementation.
    fn update_method_hashes_from_path(
        &self,
        conn: &mut SqlConnection,
        service_db_id: i32,
        path: &PathBuf,
    ) -> SetupResult<()> {
        let methods = match system_service_methods::table
            .filter(system_service_methods::system_service_id.eq(service_db_id))
            .get_results::<SystemServiceMethod>(conn)
        {
            Ok(m) => m,
            Err(_e) => {
                log::warn!(
                    "failed to find methods for service {}",
                    self.get_service_name()
                );
                return Ok(());
            }
        };

        let mut file = open_file(&path)?;
        let lexer = Lexer::new(&mut file);
        let mut parser = Parser::new(lexer);

        let class = parse_class(&mut parser).map_err(|e| SetupError::Smalisa(e.to_string()))?;

        for m in class.methods {
            let Some(dbm) = methods.iter().find(|it| {
                it.name == m.name && it.signature.as_ref().map_or(false, |s| s == m.args)
            }) else {
                continue;
            };
            self.update_method_hash(conn, dbm.id, &m)?;
        }

        Ok(())
    }

    fn update_method_hashes(
        &self,
        conn: &mut SqlConnection,
        service_db_id: i32,
        imp: &ClassSpec,
    ) -> SetupResult<()> {
        let Some(path) = self.get_class_smali_path(&imp) else {
            log::warn!("failed to find the smali file for {}", imp.name);
            return Ok(());
        };

        self.update_method_hashes_from_path(conn, service_db_id, &path)
    }

    fn update_method_hash(
        &self,
        conn: &mut SqlConnection,
        id: i32,
        method: &Method,
    ) -> SetupResult<()> {
        let hash = self.get_method_hash(method);

        update(system_service_methods::table.filter(system_service_methods::id.eq(id)))
            .set(system_service_methods::smalisa_hash.eq(&hash))
            .execute(conn)?;

        Ok(())
    }

    /// Use smalisa to get a hash of the given Method's implementation
    fn get_method_hash(&self, m: &Method) -> String {
        let mut hasher = Sha256::new();
        hasher.update(m.args);
        hash_type(&mut hasher, &m.return_type);
        for line in m.lines.iter() {
            match line {
                MethodLine::Instruction(ref ins) => {
                    let bits = ins.instruction().bits();
                    hasher.update(bits.to_be_bytes());
                    let args = ins.args();
                    match args {
                        InvArgs::TwoRegLabel(_, _, _label)
                        | InvArgs::OneRegLabel(_, _label)
                        | InvArgs::Label(_label) => {
                            // noop, don't care about labels
                        }
                        InvArgs::OneRegNum(_, s)
                        | InvArgs::TwoRegNum(_, _, s)
                        | InvArgs::RegStr(_, s) => {
                            hasher.update(s);
                        }
                        InvArgs::VarRegMethod(_, mref) => {
                            hash_method_ref(&mut hasher, mref);
                        }
                        InvArgs::OneRegField(_, fref) | InvArgs::TwoRegField(_, _, fref) => {
                            hash_field_ref(&mut hasher, fref);
                        }
                        InvArgs::OneRegClass(_, cls) | InvArgs::TwoRegClass(_, _, cls) => {
                            hash_type(&mut hasher, cls);
                        }

                        InvArgs::VarRegArray(_, arr) | InvArgs::TwoRegArray(_, _, arr) => {
                            hash_type(&mut hasher, arr);
                        }
                        InvArgs::Polymorphic(_, mref, args, ret) => {
                            hash_method_ref(&mut hasher, mref);
                            hash_type(&mut hasher, ret);
                            hasher.update(args);
                        }
                        InvArgs::Bare
                        | InvArgs::OneReg(_)
                        | InvArgs::TwoReg(_, _)
                        | InvArgs::ThreeReg(_, _, _) => {}
                    }
                }
                _ => {}
            }
        }
        let res = hasher.finalize();
        let eng = base64::engine::general_purpose::STANDARD_NO_PAD;
        eng.encode(res.as_slice())
    }

    /// Find all implementations of the $Stub file in the graph database
    ///
    /// If multiple implementations are found, there is currently no method for
    /// choosing which one to use and we just default to the first.
    fn find_and_add_impls(
        &self,
        conn: &mut SqlConnection,
        service_db_id: i32,
        stub: &ClassName,
    ) -> SetupResult<Option<ClassSpec>> {
        let graph = self.graph.ok_or_else(|| SetupError::NoGraphDatabase)?;
        let impls = graph
            .find_child_classes_of(&ClassSearch::from(stub), None)?
            .into_iter()
            .filter(|it| it.is_not_abstract())
            .collect::<Vec<ClassSpec>>();

        if impls.len() == 0 {
            log::warn!(
                "no concrete implementations for {}",
                self.get_service_name()
            );
            return Ok(None);
        }

        for imp in &impls {
            let src = imp.source.as_str();
            log::trace!(
                "{} is an implementation for {}",
                imp.name,
                self.get_service_name()
            );

            on_event!(
                &self.monitor,
                SetupEvent::FoundSystemServiceImpl {
                    service_id: self.task_id,
                    implementation: imp.name.clone(),
                    source: imp.source.clone(),
                }
            );

            let smali_name = imp.name.get_smali_name();
            self.add_system_service_impl(conn, service_db_id, smali_name.as_ref(), &src)?;
        }

        if impls.len() > 1 {
            // TODO could eventually prompt here instead
            log::warn!(
                "multiple implementations for {}, choosing first",
                self.get_service_name()
            );
        }

        Ok(impls.get(0).map(|it| it.clone()))
    }

    /// Get a hash of all methods and the path containing a valid $Stub
    ///
    /// If stub_path is given, this is straightforward. If not, we try to
    /// find the stub path based on the class name and the Context.
    fn get_methods_and_stubs_file(
        &self,
        stub: &ClassName,
    ) -> SetupResult<Option<(HashMap<String, MethodData>, PathBuf)>> {
        if let Some(p) = self.stub_path {
            return Ok(Some((self.get_methods_from_stubs_file(p)?, p.clone())));
        }

        let stub_paths = find_files_for_class(&self.ctx, stub);
        if stub_paths.len() == 0 {
            log::warn!("failed to find the smali file for stub {}", stub);
            return Ok(None);
        }
        for sf in stub_paths {
            let methods = self.get_methods_from_stubs_file(&sf)?;
            if methods.len() > 0 {
                return Ok(Some((methods, sf)));
            }
        }

        Ok(None)
    }

    /// Use smalisa to get part of the MethodData from the $Stub file
    ///
    /// This will get the names and transaction numbers of all methods, but
    /// it won't get the signatures
    fn get_methods_from_stubs_file(
        &self,
        stub_path: &PathBuf,
    ) -> SetupResult<HashMap<String, MethodData>> {
        let mut methods: HashMap<String, MethodData> = HashMap::new();
        let mut stub_file = open_file(stub_path)?;
        let stub_lexer = Lexer::new(&mut stub_file);
        let mut stub_parser = Parser::new(stub_lexer);

        let mut line = Line::default();

        // Stub file contains the TRANSACTION_{NAME} fields

        loop {
            self.cancel_check()?;
            match stub_parser.parse_line_into(&mut line) {
                Err(e) if e.is_eof() => break,
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            match &line {
                Line::Field(fld) => {
                    if fld.name.starts_with("TRANSACTION") {
                        if let Some(meta) = MethodData::from_field(fld) {
                            methods.insert(meta.name.clone(), meta);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(methods)
    }

    /// Given an interface and a stub class, add the methods
    ///
    /// If the stub_file is given, it will be used to get the methods.
    /// Otherwise we'll try to find the $Stub file from the class name
    fn add_methods_from_iface_and_stub(
        &self,
        conn: &mut SqlConnection,
        service_db_id: i32,
        iface: &ClassName,
        stub: &ClassName,
    ) -> SetupResult<()> {
        let (mut methods, stub_file) = match self.get_methods_and_stubs_file(stub)? {
            Some((m, f)) => (m, f),
            None => {
                log::info!("couldn't find stub info for {}", stub);
                return Ok(());
            }
        };

        self.try_update_method_signatures(&stub_file, iface, &mut methods)?;

        for m in methods.values() {
            self.cancel_check()?;
            on_event!(
                &self.monitor,
                SetupEvent::FoundSystemServiceMethod {
                    service_id: self.task_id,
                    name: m.name.clone(),
                    txn_id: m.txn_id,
                    signature: m.sig.clone(),
                    return_type: m.ret.clone(),
                }
            );

            let signature = m.sig.as_ref().map(|it| it.as_str());
            let return_type = m.ret.as_ref().map(|s| s.as_str());

            let ins = InsertSystemServiceMethod::new(service_db_id, m.txn_id, m.name.as_str())
                .set_signature(signature)
                .set_return_type(return_type);
            insert_into(system_service_methods::table)
                .values(&ins)
                .execute(conn)?;
        }

        Ok(())
    }

    fn add_system_service_impl(
        &self,
        conn: &mut SqlConnection,
        service_db_id: i32,
        imp: &str,
        source: &str,
    ) -> SetupResult<()> {
        let ins = InsertSystemServiceImpl::new(service_db_id, source, imp.into());
        insert_into(system_service_impls::table)
            .values(&ins)
            .execute(conn)?;
        Ok(())
    }

    /// The smalisa part of try_update_method_signatures
    fn try_update_method_signatures_inner(
        &self,
        iface_path: &PathBuf,
        methods: &mut HashMap<String, MethodData>,
    ) -> SetupResult<bool> {
        let mut mod_count = 0;

        let mut line = Line::default();
        let mut iface_file = open_file(&iface_path)?;
        let iface_lexer = Lexer::new(&mut iface_file);
        let mut iface_parser = Parser::new(iface_lexer);
        loop {
            self.cancel_check()?;
            match iface_parser.parse_line_into(&mut line) {
                Err(e) if e.is_eof() => break,
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            match &line {
                Line::MethodHeader(hdr) => {
                    if let Some(meta) = methods.get_mut(hdr.name) {
                        mod_count += 1;
                        meta.update_from_header(hdr);
                    }
                }
                _ => {}
            }
        }
        Ok(mod_count == methods.len())
    }

    /// Try to update the signature and return type for the methods
    ///
    /// This will look for the interface file right next to the $Stub file
    /// and use that to update the signature and return types with smalisa
    fn try_update_method_signatures(
        &self,
        stubs_file: &PathBuf,
        iface: &ClassName,
        methods: &mut HashMap<String, MethodData>,
    ) -> SetupResult<()> {
        // Look for the interface right next to the $Stub
        let iface_path =
            stubs_file.with_file_name(format!("{}.smali", iface.get_simple_class_name()));
        if iface_path.exists() {
            if self.try_update_method_signatures_inner(&iface_path, methods)? {
                log::trace!("updated method signatures from {:?}", iface_path);
                return Ok(());
            }
        }

        for path in find_files_for_class(&self.ctx, iface) {
            if path == iface_path {
                continue;
            }
            if self.try_update_method_signatures_inner(&path, methods)? {
                log::trace!("updated method signatures from {:?}", path);
                return Ok(());
            }
        }

        // ensure we don't have junk data
        for v in methods.values_mut() {
            v.sig = None;
            v.ret = None;
        }

        Ok(())
    }

    fn get_class_smali_path(&self, imp: &ClassSpec) -> Option<PathBuf> {
        let apk = if imp.source == FRAMEWORK_SOURCE {
            None
        } else {
            Some(DevicePath::from_squashed(&imp.source))
        };

        find_smali_file_for_class(&self.ctx, &imp.name, apk.as_ref())
    }

    #[inline]
    fn cancel_check(&self) -> SetupResult<()> {
        self.cancel.check(SetupError::Cancelled)
    }
}

impl<'a> AddApkTask<'a> {
    pub fn new(
        ctx: &'a dyn Context,
        conn: &'a mut SqlConnection,
        monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
        apktool_out_dir: &'a PathBuf,
        device_path: &'a DevicePath,
        priv_app_paths: Option<&'a HashSet<String>>,
        identifier: ApkIdentifier,
        cancel: &'a TaskCancelCheck,
    ) -> Self {
        Self {
            ctx,
            conn,
            monitor,
            cancel,
            apktool_out_dir,
            device_path,
            priv_app_paths,
            identifier,
        }
    }

    pub fn run(self) -> SetupResult<()> {
        let manifest_path = self.apktool_out_dir.join("AndroidManifest.xml");
        if !manifest_path.exists() {
            log::warn!("apk {} has no manifest", self.device_path);
            return Ok(());
        }
        let manifest = Manifest::from_file(&manifest_path).map_err(|e| {
            SetupError::InvalidManifest(self.device_path.get_device_string(), e.to_string())
        })?;

        let resolver = ApktoolManifestResolver::new(self.apktool_out_dir);

        let device_path = self.device_path.as_device_str();

        let pkg = manifest.package(&resolver);
        let is_priv = self
            .priv_app_paths
            .map(|it| it.contains(device_path))
            .unwrap_or_else(|| device_path.contains("/priv-app/"));

        // It is really unlikely for an APK to be debuggable. This is a case where it probably
        // makes more sense to just default to false when resolution fails.
        let is_debug = manifest.debuggable(&resolver).unwrap_or(false);

        let new_apk = InsertApk::new(
            &pkg,
            self.device_path.device_file_name(),
            is_debug,
            is_priv,
            self.device_path.clone(),
        );

        let apk_id = insert_into(apks::table)
            .values(&new_apk)
            .returning(apks::id)
            .get_result(self.conn)?;
        self.cancel_check()?;

        let manifest_task = AddManifestTask {
            apk_id,
            ctx: self.ctx,
            pkg,
            device_path: self.device_path,
            cancel: self.cancel,
            monitor: self.monitor,
            identifier: self.identifier,
            manifest: &manifest,
            resolver: &resolver,
        };

        manifest_task.run(self.conn)
    }

    #[inline]
    fn cancel_check(&self) -> SetupResult<()> {
        self.cancel.check(SetupError::Cancelled)
    }
}

fn hash_type(sha: &mut Sha256, ty: &Type) {
    if let Some(v) = ty.as_smali_str().as_ref() {
        sha.update(v.as_bytes());
    }
}

fn hash_method_ref(sha: &mut Sha256, mr: &MethodRef) {
    sha.update(mr.class);
    sha.update(mr.name);
    sha.update(mr.args);
    hash_type(sha, &mr.return_type);
}

fn hash_field_ref(sha: &mut Sha256, mr: &FieldRef) {
    sha.update(mr.class);
    sha.update(mr.name);
    hash_type(sha, &mr.ty);
}

fn parse_prop(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();

    let (raw_key, raw_value) = trimmed.split_once(':')?;

    let key = raw_key.trim_start_matches('[').trim_end_matches(']');
    let value = raw_value
        .trim_start()
        .trim_start_matches('[')
        .trim_end_matches(']');

    if key.len() == 0 {
        return None;
    }

    return Some((key, value));
}

struct IPCCommon<'a> {
    name: Cow<'a, str>,
    enabled: Option<bool>,
    exported: Option<bool>,
    permission: Option<Cow<'a, str>>,
}

impl<'a> IPCCommon<'a> {
    fn from_ipc(ipc: &'a dyn IPC, resolver: &dyn manifest::ManifestResolver) -> Self {
        let name = ipc.name(resolver);
        let enabled = ipc.enabled(resolver);
        let exported = ipc.exported(resolver);
        let permission = ipc.permission(resolver);

        Self {
            name,
            enabled,
            exported,
            permission,
        }
    }

    fn permission<'s>(&'s self) -> Option<&'s str> {
        self.permission.as_ref().map(|it| it.as_ref())
    }

    /// Return the parsed enabled attribute
    ///
    /// This defaults to true if resolving failed as false positives are preferred over false
    /// negatives here
    fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// Return the parsed exported attribute
    ///
    /// This defaults to true if resolving failed as false positives are preferred over false
    /// negatives here
    fn is_exported(&self) -> bool {
        self.exported.unwrap_or(true)
    }

    /// Retrieve the class and package name
    ///
    /// If the resolved name contains a `/`, we split there and derive the package name from that.
    /// Otherwise, we use the default provided package name.
    fn class_and_package<'b>(&'a self, pkg: &'b str) -> (&'a str, &'a str)
    where
        'b: 'a,
    {
        match self.name.split_once('/') {
            None => (self.name.as_ref(), pkg),
            Some((pkg, class)) => (class, pkg),
        }
    }
}

// https://source.android.com/docs/core/permissions/perms-allowlist
//
// The manual is less clear than I thought. It only mentions /vendor, /product, and
// /system explicitly, but I think anything with priv-app is a priv app. I dunno. Let's say
// thats the case.
const PRIV_APP_REGEX: &'static str = "^/(?:[^/]+/)+priv-app/.*";

fn list_priv_apps(helper: &dyn DatabaseSetupHelper) -> SetupResult<HashSet<String>> {
    let mut apps = HashSet::new();

    let re = Regex::new(PRIV_APP_REGEX).unwrap();

    let mut on_stdout = |apk_path: &str| -> anyhow::Result<()> {
        let is_priv = re.is_match(apk_path);
        if !is_priv {
            return Ok(());
        }

        apps.insert(String::from(apk_path));

        Ok(())
    };

    helper.list_packages(&mut on_stdout)?;
    Ok(apps)
}

impl<'a> DBSetupTask<'a> {
    pub fn new(
        ctx: &'a dyn Context,
        helper: &'a dyn DatabaseSetupHelper,
        graph: &'a dyn GraphDatabase,
        db: &'a DeviceDatabase,
        monitor: Option<&'a dyn EventMonitor<SetupEvent>>,
        cancel: TaskCancelCheck,
    ) -> Self {
        Self {
            ctx,
            helper,
            monitor,
            graph,
            cancel,
            db,
        }
    }

    pub fn run(&self) -> SetupResult<()> {
        log::debug!("adding apks");
        self.add_apks()?;

        let entries = self.helper.list_services()?;
        on_event!(
            &self.monitor,
            SetupEvent::DiscoveredServices {
                entries: entries.clone(),
            }
        );
        log::debug!("adding system services");
        self.add_services(&entries)?;
        log::debug!("adding device properties");
        self.add_device_props()?;
        Ok(())
    }

    fn add_device_props(&self) -> SetupResult<()> {
        let props = self.helper.get_props()?;

        let ins = props
            .iter()
            .map(|(name, value)| InsertDeviceProperty { name, value })
            .collect::<Vec<InsertDeviceProperty>>();

        let res = self
            .db
            .with_connection(|c| {
                insert_into(device_properties::table)
                    .values(ins.as_slice())
                    .execute(c)
            })
            .map_err(Error::from);

        match res {
            Err(Error::UniqueViolation(_)) => Ok(()),
            Err(e) => Err(e.into()),
            Ok(_) => Ok(()),
        }
    }

    fn add_apks(&self) -> SetupResult<()> {
        let dir = self.ctx.get_apks_dir()?.join("decompiled");
        let mut last_apk_id: usize = 0;
        let mut dir_id: usize = 0;
        last_apk_id += self.add_apks_from_dir(&dir, dir_id, last_apk_id)?;
        dir_id += 1;
        let dir = self.ctx.get_apks_dir()?.join("framework");
        self.add_apks_from_dir(&dir, dir_id, last_apk_id)?;
        Ok(())
    }

    fn add_apks_from_dir(
        &self,
        dir: &PathBuf,
        dir_id: usize,
        last_apk_id: usize,
    ) -> SetupResult<usize> {
        log::trace!("adding apks from dir: {:?}", dir);
        self.cancel_check()?;
        let files = std::fs::read_dir(&dir)?
            .filter(|r| {
                r.as_ref().map_or(false, |e| {
                    let path = e.path();
                    path.is_dir() && path_has_ext(&path, "apk")
                })
            })
            .map(|it| it.unwrap())
            .collect::<Vec<DirEntry>>();

        on_event!(
            &self.monitor,
            SetupEvent::StartedApksForDir {
                dir: dir.clone(),
                count: files.len(),
                dir_id,
            }
        );

        let res = self.add_apk_files(dir, &files, dir_id, last_apk_id);

        on_event!(
            &self.monitor,
            SetupEvent::DoneApksForDir {
                success: res.is_ok(),
                dir_id,
            }
        );

        res
    }

    fn add_apk_files(
        &self,
        dir: &PathBuf,
        files: &Vec<DirEntry>,
        dir_id: usize,
        last_apk_id: usize,
    ) -> SetupResult<usize> {
        let priv_app_names = list_priv_apps(self.helper)?;

        let mut apk_id = last_apk_id;

        for f in files {
            self.cancel_check()?;
            let identifier = ApkIdentifier { apk_id, dir_id };
            let path = f.path();
            let device_path = DevicePath::from_path(&path)?;
            log::debug!("doing apk {}", device_path);
            on_event!(
                &self.monitor,
                SetupEvent::StartAddingApk {
                    path: device_path.clone(),
                    identifier,
                }
            );
            let res = self.do_apk(&dir, &device_path, identifier, &priv_app_names);
            on_event!(
                &self.monitor,
                SetupEvent::DoneAddingApk {
                    success: res.is_ok(),
                    identifier,
                }
            );
            // Failure of a single APK shouldn't be fatal to the entire process. The event monitor
            // should ultimately notify the user that things failed somewhere.
            if let Err(e) = res {
                log::error!("failed to handle APK {}: {}", device_path, e);
            }
            // This is unconditional so we don't have to care about when the failure happened. If
            // we increment the APK ID even though one wasn't inserted it doesn't matter.
            apk_id += 1;
        }
        Ok(apk_id)
    }

    fn do_apk(
        &self,
        decompiled_dir: &PathBuf,
        device_path: &DevicePath,
        identifier: ApkIdentifier,
        priv_app_names: &HashSet<String>,
    ) -> SetupResult<()> {
        let apktool_out_dir = decompiled_dir.join(device_path);

        let ctx = self.ctx;
        let monitor = self.monitor;
        let cancel = &self.cancel;

        self.db.with_transaction(|conn| {
            let task = AddApkTask::new(
                ctx,
                conn,
                monitor,
                &apktool_out_dir,
                device_path,
                Some(priv_app_names),
                identifier,
                cancel,
            );

            task.run()
        })
    }

    #[inline]
    fn cancel_check(&self) -> SetupResult<()> {
        self.cancel.check(SetupError::Cancelled)
    }

    fn add_services(&self, services: &Vec<ServiceMeta>) -> SetupResult<()> {
        for (monitor_service_id, s) in services.iter().enumerate() {
            self.cancel_check()?;

            let mut task = AddSystemServiceTask::new(
                self.ctx,
                monitor_service_id,
                &s,
                Some(self.graph),
                self.db,
                self.monitor,
                &self.cancel,
            );
            // We want to catch the unique violation here instead of in
            // the task so we don't redo a bunch of extra work.
            task.allow_exists = false;
            match task.run() {
                Err(SetupError::DB(Error::UniqueViolation(_))) => {}
                Err(e) => return Err(e),
                Ok(_) => {}
            }
        }

        Ok(())
    }
}

struct MethodData {
    name: String,
    txn_id: i32,
    sig: Option<String>,
    ret: Option<String>,
}

impl MethodData {
    fn update_from_header(&mut self, hdr: &MethodHeader) {
        self.sig = Some(String::from(hdr.args));
        self.ret = Some(hdr.return_type.to_string());
    }

    fn from_field(field: &smalisa::Field) -> Option<Self> {
        let name = String::from(field.name.strip_prefix("TRANSACTION_")?);
        let txn_id = match field.raw_value.to_literal()? {
            Literal::Int(val) => val,
            _ => {
                log::warn!("invalid value for field: {:?}", field.raw_value);
                return None;
            }
        };

        Some(Self {
            name,
            txn_id,
            sig: None,
            ret: None,
        })
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ServiceMeta {
    pub service_name: String,
    pub iface: Option<ClassName>,
}

impl ServiceMeta {
    fn from_line(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut split = trimmed.split_ascii_whitespace();
        let _num = str::parse::<usize>(split.next()?).ok()?;
        let name = split.next()?.trim_end_matches(':');
        let iface = split.next()?.trim_end_matches(']').trim_start_matches('[');
        Some(Self {
            service_name: String::from(name),
            iface: if iface.is_empty() {
                None
            } else {
                Some(ClassName::from(iface))
            },
        })
    }
}

fn parse_device_props_output(output: &str) -> HashMap<String, String> {
    let mut props = HashMap::new();
    let mut multiline = String::new();

    for line in output.split_terminator('\n') {
        // Still in a single line
        if multiline.len() == 0 {
            // Looks like a property
            if line.starts_with('[') {
                if line.ends_with(']') {
                    log::trace!("Property line: {}", line);
                    if let Some((k, v)) = parse_prop(line) {
                        props.insert(String::from(k), String::from(v));
                        continue;
                    } else {
                        log::warn!(
                            "invalid getprop output parse_prop failed, dropping: {}",
                            line
                        );
                    }
                } else {
                    multiline.push_str(line);
                    multiline.push('\n');
                }
            } else {
                log::warn!("invalid getprop output (no opening [), dropping: {}", line);
            }
        } else {
            // Keep on adding
            multiline.push_str(line);
            // The end of  the value, great
            if line.ends_with(']') {
                log::trace!("Property line multiline: {}", multiline);
                if let Some((k, v)) = parse_prop(&multiline) {
                    props.insert(String::from(k), String::from(v));
                } else {
                    log::warn!(
                        "invalid getprop output in multiline, dropping: {}",
                        multiline
                    );
                }
                multiline.clear();
            } else {
                multiline.push('\n');
            }
        }
    }
    props
}

pub fn get_project_dbsetup_helper(
    ctx: &dyn Context,
) -> crate::Result<Box<dyn DatabaseSetupHelper>> {
    let config = match ctx.get_project_config()? {
        None => return Ok(Box::new(AdbDatabaseSetupHelper::new(ExecAdb::new(ctx)?))),
        Some(v) => v,
    };

    let base = config.get_map();

    let cfg = match base.maybe_get_map_typecheck("device-access")? {
        None => return Ok(Box::new(AdbDatabaseSetupHelper::new(ExecAdb::new(ctx)?))),
        Some(v) => v,
    };

    if cfg.has(FSDumpAccess::CONFIG_KEY) && cfg.has(ADB_CONFIG_KEY) {
        return Err(config.invalid_error(format!(
            "`device-access` can't have both `{ADB_CONFIG_KEY}` and `{}` keys",
            FSDumpAccess::CONFIG_KEY
        )));
    }

    match cfg.maybe_get_map_typecheck(FSDumpAccess::CONFIG_KEY)? {
        Some(v) => Ok(Box::new(FSDumpAccess::from_cfg_map(ctx, &v)?)),
        None => match cfg.maybe_get_map_typecheck(ADB_CONFIG_KEY)? {
            Some(v) => Ok(Box::new(AdbDatabaseSetupHelper::new(ExecAdb::from_config(
                ctx, &v,
            )?))),
            None => Err(config.invalid_error(format!(
                "`device-access` needs either `{ADB_CONFIG_KEY}` or `{}` keys",
                FSDumpAccess::CONFIG_KEY
            ))),
        },
    }
}

pub struct AdbDatabaseSetupHelper<T: Adb>(T);

impl<T> Deref for AdbDatabaseSetupHelper<T>
where
    T: Adb,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> AdbDatabaseSetupHelper<T>
where
    T: Adb,
{
    pub fn new(adb: T) -> Self {
        Self(adb)
    }
}

impl<T> AdbDatabaseSetupHelper<T>
where
    T: Adb,
{
    pub fn as_adb(&self) -> &dyn Adb {
        &self.0
    }
}

impl<T> DatabaseSetupHelper for Box<T>
where
    T: DatabaseSetupHelper + ?Sized,
{
    fn get_props(&self) -> crate::Result<HashMap<String, String>> {
        self.as_ref().get_props()
    }

    fn list_services(&self) -> crate::Result<Vec<ServiceMeta>> {
        self.as_ref().list_services()
    }

    fn list_packages(&self, on_pkg: &mut PackageCallback) -> crate::Result<()> {
        self.as_ref().list_packages(on_pkg)
    }
}

impl<T> DatabaseSetupHelper for AdbDatabaseSetupHelper<T>
where
    T: Adb,
{
    fn get_props(&self) -> crate::Result<HashMap<String, String>> {
        let cmdout = self.shell("getprop")?.err_on_status()?;
        let output = cmdout.stdout_utf8_lossy();
        Ok(parse_device_props_output(output.as_ref()))
    }

    fn list_services(&self) -> crate::Result<Vec<ServiceMeta>> {
        let mut into = Vec::new();
        let mut on_stdout = |line: &str| {
            match ServiceMeta::from_line(line) {
                Some(it) => into.push(it),
                None => {}
            };
            Ok(())
        };
        let mut on_stderr = |line: &str| {
            log::warn!("{}", line);
            Ok(())
        };
        err_on_status(self.shell_split_streamed(
            "service list",
            b'\n',
            &mut on_stdout,
            &mut on_stderr,
        )?)?;
        Ok(into)
    }

    fn list_packages(&self, on_pkg: &mut PackageCallback) -> crate::Result<()> {
        let mut on_stdout = |line: &str| -> anyhow::Result<()> {
            let line = line.trim();
            let start = match line.find(':') {
                Some(v) => v + 1,
                None => {
                    log::error!("invalid output for list packages (no `:`): {}", line);
                    return Ok(());
                }
            };
            let end = match line.rfind('=') {
                Some(v) => v,
                None => {
                    log::error!("invalid output for list packages (no `=`): {}", line);
                    return Ok(());
                }
            };

            let apk_path = &line[start..end];
            if apk_path.len() == 0 {
                log::warn!("apk missing path for line: {}", line);
                return Ok(());
            }

            on_pkg(apk_path)
        };
        let mut on_stderr = |line: &str| -> anyhow::Result<()> {
            log::error!("{}", line);
            Ok(())
        };
        err_on_status(self.shell_split_streamed(
            "pm list packages -s -f",
            b'\n',
            &mut on_stdout,
            &mut on_stderr,
        )?)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::process::ExitStatus;

    use crate::{
        command::CmdOutput,
        testing::{mock_adb, MockAdb},
    };

    use super::*;
    use rstest::*;

    #[rstest]
    fn test_parse_getprop_output() {
        let output = r#"
[foo.bar.baz]: [wowza]
[ok.great]: []
[multiple.lines]: [because
why
not
]
[this.prop]: [
hmm]
[still.good]: [1]
"#;

        let props = parse_device_props_output(output);

        let expected_props: &[(String, String)] = &[
            ("foo.bar.baz".into(), "wowza".into()),
            ("ok.great".into(), "".into()),
            ("multiple.lines".into(), "because\nwhy\nnot\n".into()),
            ("this.prop".into(), "\nhmm".into()),
            ("still.good".into(), "1".into()),
        ];
        let mut expected: HashMap<String, String> = HashMap::new();
        for (k, v) in expected_props {
            expected.insert(k.into(), v.into());
        }

        assert_eq!(props, expected);
    }

    #[rstest]
    fn test_service_entry_by_line() {
        let line = "203	media.drm: [android.media.IMediaDrmService]";
        let parsed =
            ServiceMeta::from_line(line).expect(&format!("should succeed in parsing {}", line));
        assert_eq!(
            parsed.service_name, "media.drm",
            "service name incorrect for {}",
            line
        );
        assert_eq!(
            parsed.iface,
            Some("android.media.IMediaDrmService".into()),
            "iface incorrect for {}",
            line
        );
        let line = "204	installd: []";
        let parsed =
            ServiceMeta::from_line(line).expect(&format!("should succeed in parsing {}", line));
        assert_eq!(
            parsed.service_name, "installd",
            "service name incorrect for {}",
            line
        );
        assert_eq!(parsed.iface, None, "iface incorrect for {}", line);

        // No number at the start
        assert!(ServiceMeta::from_line("media.drm: [android.media.IMediaDrmService]").is_none(),);
    }

    #[rstest]
    fn test_parse_property() {
        assert_eq!(
            parse_prop("[foo.bar.baz]: [32]"),
            Some(("foo.bar.baz", "32"))
        );
        assert_eq!(parse_prop("not a property"), None);
    }

    #[rstest]
    fn test_adb_get_props(mut mock_adb: MockAdb) {
        let props: &[(&str, &str)] = &[
            ("apex.all.ready", "true"),
            ("bluetooth.device.class_of_device", "90,2,12"),
            ("bluetooth.profile.vcp.controller.enabled", "false"),
            ("bootreceiver.enable", "1"),
            ("build.version.extensions.ad_services", "17"),
            ("cache_key.bluetooth.bluetooth_adapter_get_connection_state", "-8292402961925351002"),
            ("dalvik.vm.appimageformat", "lz4"),
            ("dalvik.vm.dex2oat-Xms", "64m"),
            ("debug.tracing.device_state", "0:DEFAULT"),
            ("debug.tracing.screen_brightness", "0.39763778"),
            ("gsm.version.baseband", "1.0.0.0"),
            ("init.svc.android-hardware-media-c2-goldfish-hal-1-0", "running"),
            ("persist.device_config.runtime_native.metrics.reporting-spec", "1,5,30,60,600"),
            ("persist.sys.boot.reason.history", "reboot,1753125763\nreboot,factory_reset,1753122839\nreboot,1753122820"),
            ("remote_provisioning.hostname", "remoteprovisioning.googleapis.com"),
            ("ro.boot.boot_devices", "pci0000:00/0000:00:03.0 pci0000:00/0000:00:06.0"),
            ("ro.boot.logcat", "*:V"),
            ("ro.boot.qemu.adb.pubkey", "A= nsa@nsa"),
            ("ro.boot.qemu.avd_name", "Pixel_9_36"),
            ("ro.build.description", "sdk_gphone64_x86_64-user 16 BP22.250325.006 13344233 release-keys"),
            ("ro.build.display.id", "BP22.250325.006"),
            ("ro.build.fingerprint", "google/sdk_gphone64_x86_64/emu64xa:16/BP22.250325.006/13344233:user/release-keys"),
            ("ro.build.version.known_codenames", "Base,Base11,Cupcake,Donut,Eclair,Eclair01,EclairMr1,Froyo,Gingerbread,GingerbreadMr1,Honeycomb,HoneycombMr1,HoneycombMr2,IceCreamSandwich,IceCreamSandwichMr1,JellyBean,JellyBeanMr1,JellyBeanMr2,Kitkat,KitkatWatch,Lollipop,LollipopMr1,M,N,NMr1,O,OMr1,P,Q,R,S,Sv2,Tiramisu,UpsideDownCake,VanillaIceCream,Baklava"),
            ("ro.system_dlkm.build.fingerprint", "google/sdk_gphone64_x86_64/emu64xa:16/BP22.250325.006/13344233:user/release-keys"),
            ("ro.wifi.channels", ""),
        ];
        let mut expected: HashMap<String, String> = HashMap::new();

        for (k, v) in props {
            expected.insert((*k).into(), (*v).into());
        }

        let emu_output = r#"[apex.all.ready]: [true]
[bluetooth.device.class_of_device]: [90,2,12]
[bluetooth.profile.vcp.controller.enabled]: [false]
[bootreceiver.enable]: [1]
[build.version.extensions.ad_services]: [17]
[cache_key.bluetooth.bluetooth_adapter_get_connection_state]: [-8292402961925351002]
[dalvik.vm.appimageformat]: [lz4]
[dalvik.vm.dex2oat-Xms]: [64m]
[debug.tracing.device_state]: [0:DEFAULT]
[debug.tracing.screen_brightness]: [0.39763778]
[gsm.version.baseband]: [1.0.0.0]
[init.svc.android-hardware-media-c2-goldfish-hal-1-0]: [running]
[persist.device_config.runtime_native.metrics.reporting-spec]: [1,5,30,60,600]
[persist.sys.boot.reason.history]: [reboot,1753125763
reboot,factory_reset,1753122839
reboot,1753122820]
[remote_provisioning.hostname]: [remoteprovisioning.googleapis.com]
[ro.boot.boot_devices]: [pci0000:00/0000:00:03.0 pci0000:00/0000:00:06.0]
[ro.boot.logcat]: [*:V]
[ro.boot.qemu.adb.pubkey]: [A= nsa@nsa]
[ro.boot.qemu.avd_name]: [Pixel_9_36]
[ro.build.description]: [sdk_gphone64_x86_64-user 16 BP22.250325.006 13344233 release-keys]
[ro.build.display.id]: [BP22.250325.006]
[ro.build.fingerprint]: [google/sdk_gphone64_x86_64/emu64xa:16/BP22.250325.006/13344233:user/release-keys]
[ro.build.version.known_codenames]: [Base,Base11,Cupcake,Donut,Eclair,Eclair01,EclairMr1,Froyo,Gingerbread,GingerbreadMr1,Honeycomb,HoneycombMr1,HoneycombMr2,IceCreamSandwich,IceCreamSandwichMr1,JellyBean,JellyBeanMr1,JellyBeanMr2,Kitkat,KitkatWatch,Lollipop,LollipopMr1,M,N,NMr1,O,OMr1,P,Q,R,S,Sv2,Tiramisu,UpsideDownCake,VanillaIceCream,Baklava]
[ro.system_dlkm.build.fingerprint]: [google/sdk_gphone64_x86_64/emu64xa:16/BP22.250325.006/13344233:user/release-keys]
[ro.wifi.channels]: []"#;

        mock_adb.expect_shell().returning(move |_| {
            let mut stdout: Vec<u8> = Vec::with_capacity(emu_output.len());
            stdout.extend(emu_output.as_bytes());

            Ok(CmdOutput {
                status: ExitStatus::default(),
                stdout,
                stderr: Vec::new(),
            })
        });

        let helper = AdbDatabaseSetupHelper::new(mock_adb);
        let props = helper.get_props().unwrap();
        assert_eq!(props, expected);
    }

    #[rstest]
    fn test_adb_list_packages(mut mock_adb: MockAdb) {
        let packages = vec![
            "package:/apex/com.android.uwb/priv-app/ServiceUwbResourcesGoogle@360526040/ServiceUwbResourcesGoogle.apk=com.google.android.uwb.resources",
            "package:/product/priv-app/KidsSupervisionStub/KidsSupervisionStub.apk=com.google.android.gms.supervision",
            "package:/system_ext/priv-app/WallpaperPickerGoogleRelease/WallpaperPickerGoogleRelease.apk=com.google.android.apps.wallpaper",
            "package:/apex/com.android.cellbroadcast/priv-app/GoogleCellBroadcastServiceModule@360526020/GoogleCellBroadcastServiceModule.apk=com.google.android.cellbroadcastservice",
            "package:/system/priv-app/TagGoogle/TagGoogle.apk=com.google.android.tag"
        ];

        mock_adb
            .expect_shell_split_streamed()
            .returning(move |_, _, on_stdout_line, _| {
                for p in packages.iter() {
                    on_stdout_line(p).unwrap();
                }
                Ok(ExitStatus::default())
            });

        let mut pkgs = Vec::new();

        let mut on_pkg = |pkg: &str| {
            pkgs.push(String::from(pkg));
            Ok(())
        };

        let expected = vec![
            "/apex/com.android.uwb/priv-app/ServiceUwbResourcesGoogle@360526040/ServiceUwbResourcesGoogle.apk",
            "/product/priv-app/KidsSupervisionStub/KidsSupervisionStub.apk",
            "/system_ext/priv-app/WallpaperPickerGoogleRelease/WallpaperPickerGoogleRelease.apk",
            "/apex/com.android.cellbroadcast/priv-app/GoogleCellBroadcastServiceModule@360526020/GoogleCellBroadcastServiceModule.apk",
            "/system/priv-app/TagGoogle/TagGoogle.apk"
        ];

        let helper = AdbDatabaseSetupHelper::new(mock_adb);
        helper.list_packages(&mut on_pkg).unwrap();
        assert_eq!(pkgs, expected);
    }

    #[rstest]
    fn test_list_privapps_adb(mut mock_adb: MockAdb) {
        let emulator_output = r#"package:/product/overlay/SystemUIEmulationPixel3XL/SystemUIEmulationPixel3XLOverlay.apk=com.android.systemui.emulation.pixel_3_xl
package:/apex/com.android.uwb/priv-app/ServiceUwbResourcesGoogle@360526040/ServiceUwbResourcesGoogle.apk=com.google.android.uwb.resources
package:/product/overlay/SystemUIEmulationPixelFold/SystemUIEmulationPixelFoldOverlay.apk=com.android.systemui.emulation.pixel_fold
package:/system/app/BookmarkProvider/BookmarkProvider.apk=com.android.bookmarkprovider
package:/product/priv-app/KidsSupervisionStub/KidsSupervisionStub.apk=com.google.android.gms.supervision
package:/product/overlay/BuiltInPrintService__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.bips.auto_generated_rro_product__
package:/system_ext/priv-app/WallpaperPickerGoogleRelease/WallpaperPickerGoogleRelease.apk=com.google.android.apps.wallpaper
package:/product/app/Camera2/Camera2.apk=com.android.camera2
package:/product/overlay/SystemUIEmulationPixel7Pro/SystemUIEmulationPixel7ProOverlay.apk=com.android.systemui.emulation.pixel_7_pro
package:/apex/com.android.cellbroadcast/priv-app/GoogleCellBroadcastServiceModule@360526020/GoogleCellBroadcastServiceModule.apk=com.google.android.cellbroadcastservice
package:/system/priv-app/TagGoogle/TagGoogle.apk=com.google.android.tag
package:/product/overlay/CompanionDeviceManager__emulator__auto_generated_characteristics_rro.apk=com.android.companiondevicemanager.auto_generated_characteristics_rro
package:/product/overlay/EmulationPixel4XL/EmulationPixel4XLOverlay.apk=com.android.internal.emulation.pixel_4_xl
package:/vendor/overlay/goldfish_overlay_connectivity_google.apk=com.google.android.connectivity.resources.goldfish.overlay
package:/product/overlay/StorageManager__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.storagemanager.auto_generated_rro_product__
package:/system/priv-app/UserDictionaryProvider/UserDictionaryProvider.apk=com.android.providers.userdictionary
package:/system/priv-app/GooglePackageInstaller/GooglePackageInstaller.apk=com.google.android.packageinstaller
package:/product/priv-app/GoogleDialer/GoogleDialer.apk=com.google.android.dialer
package:/system/priv-app/BuiltInPrintService/BuiltInPrintService.apk=com.android.bips
package:/product/overlay/NavigationBarMode3Button/NavigationBarMode3ButtonOverlay.apk=com.android.internal.systemui.navbar.threebutton
package:/product/overlay/EmulationPixel9/EmulationPixel9Overlay.apk=com.android.internal.emulation.pixel_9
package:/system/app/KeyChain/KeyChain.apk=com.android.keychain
package:/apex/com.android.permission/priv-app/GooglePermissionController@360526020/GooglePermissionController.apk=com.google.android.permissioncontroller
package:/system/priv-app/LiveWallpapersPicker/LiveWallpapersPicker.apk=com.android.wallpaper.livepicker
package:/product/overlay/framework-res__emulator__auto_generated_characteristics_rro.apk=android.auto_generated_characteristics_rro
package:/system/app/CaptivePortalLoginGoogle/CaptivePortalLoginGoogle.apk=com.google.android.captiveportallogin
package:/system/priv-app/SoundPicker/SoundPicker.apk=com.android.soundpicker
package:/product/overlay/DisplayCutoutEmulationCorner/DisplayCutoutEmulationCornerOverlay.apk=com.android.internal.display.cutout.emulation.corner
package:/product/priv-app/AndroidAutoStubPrebuilt/AndroidAutoStubPrebuilt.apk=com.google.android.projection.gearhead
package:/product/overlay/SystemUIEmulationPixel4a/SystemUIEmulationPixel4aOverlay.apk=com.android.systemui.emulation.pixel_4a
package:/system/app/BasicDreams/BasicDreams.apk=com.android.dreams.basic
package:/product/overlay/SystemUIGoogle__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.systemui.auto_generated_rro_product__
package:/product/overlay/framework-res__sdk_gphone64_x86_64__auto_generated_rro_product.apk=android.auto_generated_rro_product__
package:/apex/com.android.ondevicepersonalization/priv-app/OnDevicePersonalizationGoogle@360526000/OnDevicePersonalizationGoogle.apk=com.google.android.ondevicepersonalization.services
package:/system/priv-app/ExternalStorageProvider/ExternalStorageProvider.apk=com.android.externalstorage
package:/product/app/PrebuiltGmail/PrebuiltGmail.apk=com.google.android.gm
package:/system/priv-app/NetworkStackGoogle/NetworkStackGoogle.apk=com.google.android.networkstack
package:/product/overlay/EmulationPixel9Pro/EmulationPixel9ProOverlay.apk=com.android.internal.emulation.pixel_9_pro
package:/apex/com.android.compos/app/CompOSPayloadApp@BP22.250325.006/CompOSPayloadApp.apk=com.android.compos.payload
package:/apex/com.android.rkpd/priv-app/rkpdapp.google@360526000/rkpdapp.google.apk=com.google.android.rkpdapp
package:/product/app/Photos/Photos.apk=com.google.android.apps.photos
package:/product/overlay/EmulatorTetheringConfigOverlay.apk=com.android.networkstack.tethering.emulator
package:/product/overlay/LargeScreenSettingsProviderOverlay.apk=com.google.android.overlay.largescreensettingsprovider
package:/product/overlay/SystemUIEmulationPixel3/SystemUIEmulationPixel3Overlay.apk=com.android.systemui.emulation.pixel_3
package:/system/app/PrintSpooler/PrintSpooler.apk=com.android.printspooler
package:/product/overlay/SystemUIEmulationPixel8a/SystemUIEmulationPixel8aOverlay.apk=com.android.systemui.emulation.pixel_8a
package:/apex/com.android.healthfitness/priv-app/HealthConnectControllerGoogle@360526040/HealthConnectControllerGoogle.apk=com.google.android.healthconnect.controller
package:/system_ext/app/AccessibilityMenu/AccessibilityMenu.apk=com.android.systemui.accessibility.accessibilitymenu
package:/apex/com.android.apex.cts.shim/app/CtsShim@MAIN/CtsShim.apk=com.android.cts.ctsshim
package:/product/overlay/SystemUIEmulationPixel6a/SystemUIEmulationPixel6aOverlay.apk=com.android.systemui.emulation.pixel_6a
package:/product/overlay/SystemUIEmulationPixel7/SystemUIEmulationPixel7Overlay.apk=com.android.systemui.emulation.pixel_7
package:/system_ext/priv-app/QuickAccessWallet/QuickAccessWallet.apk=com.android.systemui.plugin.globalactions.wallet
package:/apex/com.android.nfcservices/priv-app/NfcNciApexMigrationGoogle@360526020/NfcNciApexMigrationGoogle.apk=com.android.nfc
package:/data/app/~~KRnzd5UsNkW-B-jh_uEMDQ==/com.google.android.webview-45A-UrUAaUQRRc0hu6UAOw==/WebViewGoogle.apk=com.google.android.webview
package:/product/priv-app/PrebuiltBugle/PrebuiltBugle.apk=com.google.android.apps.messaging
package:/system_ext/priv-app/NexusLauncherRelease/NexusLauncherRelease.apk=com.google.android.apps.nexuslauncher
package:/apex/com.android.apex.cts.shim/priv-app/CtsShimPriv@MAIN/CtsShimPriv.apk=com.android.cts.priv.ctsshim
package:/product/overlay/SystemUIEmulationPixel4XL/SystemUIEmulationPixel4XLOverlay.apk=com.android.systemui.emulation.pixel_4_xl
package:/product/app/CalendarGooglePrebuilt/CalendarGooglePrebuilt.apk=com.google.android.calendar
package:/vendor/overlay/EmulatorTalkBackOverlay/EmulatorTalkBackOverlay.apk=com.google.android.marvin.talkbackoverlay
package:/product/overlay/UwbGoogleOverlay.apk=com.google.android.uwb.resources.goldfish.overlay
package:/apex/com.android.mediaprovider/priv-app/MediaProviderGoogle@360526000/MediaProviderGoogle.apk=com.google.android.providers.media.module
package:/system/priv-app/TelephonyProvider/TelephonyProvider.apk=com.android.providers.telephony
package:/product/overlay/EmulationPixel7/EmulationPixel7Overlay.apk=com.android.internal.emulation.pixel_7
package:/apex/com.android.wifi/app/OsuLoginGoogle@360526000/OsuLoginGoogle.apk=com.google.android.hotspot2.osulogin
package:/system/priv-app/BlockedNumberProvider/BlockedNumberProvider.apk=com.android.providers.blockednumber
package:/system/app/GoogleBluetoothLegacyMigration/GoogleBluetoothLegacyMigration.apk=com.android.bluetooth
package:/system/app/WallpaperBackup/WallpaperBackup.apk=com.android.wallpaperbackup
package:/product/overlay/EmulationPixel3a/EmulationPixel3aOverlay.apk=com.android.internal.emulation.pixel_3a
package:/system/priv-app/CallLogBackup/CallLogBackup.apk=com.android.calllogbackup
package:/product/priv-app/SettingsIntelligenceGooglePrebuilt/SettingsIntelligenceGooglePrebuilt.apk=com.google.android.settings.intelligence
package:/apex/com.android.ondevicepersonalization/app/FederatedComputeGoogle@360526000/FederatedComputeGoogle.apk=com.google.android.federatedcompute
package:/apex/com.android.adservices/app/SdkSandboxGoogle@360526040/SdkSandboxGoogle.apk=com.google.android.sdksandbox
package:/system_ext/priv-app/GoogleSdkSetup/GoogleSdkSetup.apk=com.google.android.googlesdksetup
package:/product/overlay/EmulationPixel3XL/EmulationPixel3XLOverlay.apk=com.android.internal.emulation.pixel_3_xl
package:/system_ext/priv-app/AvatarPickerGoogle/AvatarPickerGoogle.apk=com.google.android.avatarpicker
package:/system_ext/priv-app/ThemePicker/ThemePicker.apk=com.android.wallpaper
package:/product/overlay/EmulationPixel9ProFold/EmulationPixel9ProFoldOverlay.apk=com.android.internal.emulation.pixel_9_pro_fold
package:/system/priv-app/TeleService/TeleService.apk=com.android.phone
package:/product/app/MarkupGoogle_v2/MarkupGoogle_v2.apk=com.google.android.markup
package:/system_ext/priv-app/EmulatorRadioConfig/EmulatorRadioConfig.apk=com.android.emulator.radio.config
package:/product/overlay/GoogleWebViewOverlay.apk=com.google.android.overlay.googlewebview
package:/product/priv-app/ImsServiceEntitlement/ImsServiceEntitlement.apk=com.android.imsserviceentitlement
package:/product/overlay/SystemUIEmulationPixel9ProXL/SystemUIEmulationPixel9ProXLOverlay.apk=com.android.systemui.emulation.pixel_9_pro_xl
package:/apex/com.android.wifi/priv-app/ServiceWifiResourcesGoogle@360526000/ServiceWifiResourcesGoogle.apk=com.google.android.wifi.resources
package:/system/app/CompanionDeviceManager/CompanionDeviceManager.apk=com.android.companiondevicemanager
package:/product/overlay/EmulationPixel2XL/EmulationPixel2XLOverlay.apk=com.android.internal.emulation.pixel_2_xl
package:/product/overlay/ContactsProvider__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.providers.contacts.auto_generated_rro_product__
package:/system/app/Stk/Stk.apk=com.android.stk
package:/system/priv-app/IntentResolver/IntentResolver.apk=com.android.intentresolver
package:/system/priv-app/MusicFX/MusicFX.apk=com.android.musicfx
package:/system/priv-app/MtpService/MtpService.apk=com.android.mtp
package:/system/priv-app/CalendarProvider/CalendarProvider.apk=com.android.providers.calendar
package:/system/app/BluetoothMidiService/BluetoothMidiService.apk=com.android.bluetoothmidiservice
package:/product/overlay/AccessibilityMenu__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.systemui.accessibility.accessibilitymenu.auto_generated_rro_product__
package:/product/app/GoogleTTS/GoogleTTS.apk=com.google.android.tts
package:/product/overlay/SystemUIEmulationPixel9/SystemUIEmulationPixel9Overlay.apk=com.android.systemui.emulation.pixel_9
package:/apex/com.android.wifi/app/WifiDialogGoogle@360526000/WifiDialogGoogle.apk=com.google.android.wifi.dialog
package:/system/priv-app/SharedStorageBackup/SharedStorageBackup.apk=com.android.sharedstoragebackup
package:/product/priv-app/GoogleRestorePrebuilt-v717308/GoogleRestorePrebuilt-v717308.apk=com.google.android.apps.restore
package:/product/overlay/DisplayCutoutEmulationWaterfall/DisplayCutoutEmulationWaterfallOverlay.apk=com.android.internal.display.cutout.emulation.waterfall
package:/product/app/PixelThemesStub/PixelThemesStub.apk=com.google.android.apps.customization.pixel
package:/product/overlay/ManagedProvisioning__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.managedprovisioning.auto_generated_rro_product__
package:/product/priv-app/OdadPrebuilt/OdadPrebuilt.apk=com.google.android.odad
package:/product/app/PrebuiltDeskClockGoogle/PrebuiltDeskClockGoogle.apk=com.google.android.deskclock
package:/product/overlay/EmulationPixel8a/EmulationPixel8aOverlay.apk=com.android.internal.emulation.pixel_8a
package:/product/overlay/EmulationPixel3aXL/EmulationPixel3aXLOverlay.apk=com.android.internal.emulation.pixel_3a_xl
package:/apex/com.android.tethering/priv-app/ServiceConnectivityResourcesGoogle@360526040/ServiceConnectivityResourcesGoogle.apk=com.google.android.connectivity.resources
package:/system/priv-app/LocalTransport/LocalTransport.apk=com.android.localtransport
package:/product/overlay/SystemUIEmulationPixel9a/SystemUIEmulationPixel9aOverlay.apk=com.android.systemui.emulation.pixel_9a
package:/system/priv-app/InputDevices/InputDevices.apk=com.android.inputdevices
package:/product/overlay/TeleService__emulator__auto_generated_characteristics_rro.apk=com.android.phone.auto_generated_characteristics_rro
package:/system/app/CameraExtensionsProxy/CameraExtensionsProxy.apk=com.android.cameraextensions
package:/system/priv-app/DownloadProvider/DownloadProvider.apk=com.android.providers.downloads
package:/system_ext/priv-app/WallpaperCropper/WallpaperCropper.apk=com.android.wallpapercropper
package:/system/priv-app/DeviceDiagnostics/DeviceDiagnostics.apk=com.android.devicediagnostics
package:/product/overlay/EmulationPixel5/EmulationPixel5Overlay.apk=com.android.internal.emulation.pixel_5
package:/system/priv-app/ONS/ONS.apk=com.android.ons
package:/product/overlay/SettingsProvider__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.providers.settings.auto_generated_rro_product__
package:/system/priv-app/ProxyHandler/ProxyHandler.apk=com.android.proxyhandler
package:/apex/com.android.nfcservices/priv-app/NfcNciApexGoogle@360526020/NfcNciApexGoogle.apk=com.google.android.nfc
package:/apex/com.android.healthfitness/app/HealthConnectBackupRestoreGoogle@360526040/HealthConnectBackupRestoreGoogle.apk=com.google.android.health.connect.backuprestore
package:/product/overlay/EmulationPixel7Pro/EmulationPixel7ProOverlay.apk=com.android.internal.emulation.pixel_7_pro
package:/product/app/ModuleMetadataGoogle/ModuleMetadataGoogle.apk=com.google.android.modulemetadata
package:/product/priv-app/GoogleOneTimeInitializer/GoogleOneTimeInitializer.apk=com.google.android.onetimeinitializer
package:/system/priv-app/MmsService/MmsService.apk=com.android.mms.service
package:/product/overlay/DeviceDiagnostics__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.devicediagnostics.auto_generated_rro_product__
package:/system/app/EasterEgg/EasterEgg.apk=com.android.egg
package:/product/overlay/RanchuCommonOverlay.apk=com.android.internal.ranchu.commonoverlay
package:/system_ext/app/EmergencyInfoGoogleNoUi/EmergencyInfoGoogleNoUi.apk=com.android.emergency
package:/product/overlay/DisplayCutoutEmulationHole/DisplayCutoutEmulationHoleOverlay.apk=com.android.internal.display.cutout.emulation.hole
package:/product/overlay/DisplayCutoutEmulationDouble/DisplayCutoutEmulationDoubleOverlay.apk=com.android.internal.display.cutout.emulation.double
package:/system/app/PacProcessor/PacProcessor.apk=com.android.pacprocessor
package:/system/priv-app/ManagedProvisioning/ManagedProvisioning.apk=com.android.managedprovisioning
package:/system/priv-app/Shell/Shell.apk=com.android.shell
package:/product/overlay/EmulationPixel7a/EmulationPixel7aOverlay.apk=com.android.internal.emulation.pixel_7a
package:/system_ext/priv-app/GoogleFeedback/GoogleFeedback.apk=com.google.android.feedback
package:/system/app/GoogleExtShared/GoogleExtShared.apk=com.google.android.ext.shared
package:/system/app/PartnerBookmarksProvider/PartnerBookmarksProvider.apk=com.android.providers.partnerbookmarks
package:/product/overlay/FontNotoSerifSource/FontNotoSerifSourceOverlay.apk=com.android.theme.font.notoserifsource
package:/apex/com.android.virt/app/android.system.virtualmachine.res@BP22.250325.006/android.system.virtualmachine.res.apk=com.android.virtualmachine.res
package:/product/priv-app/DeviceIntelligenceNetworkPrebuilt-astrea_20240329.00_RC02/DeviceIntelligenceNetworkPrebuilt-astrea_20240329.00_RC02.apk=com.google.android.as.oss
package:/product/overlay/NotesRoleEnabled/NotesRoleEnabledOverlay.apk=com.android.role.notes.enabled
package:/apex/com.android.cellbroadcast/priv-app/GoogleCellBroadcastApp@360526020/GoogleCellBroadcastApp.apk=com.google.android.cellbroadcastreceiver
package:/product/overlay/TelephonyProvider__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.providers.telephony.auto_generated_rro_product__
package:/product/overlay/LargeScreenConfigOverlay.apk=com.google.android.overlay.largescreenconfig
package:/system/priv-app/SettingsProvider/SettingsProvider.apk=com.android.providers.settings
package:/product/overlay/SystemUIEmulationPixel8/SystemUIEmulationPixel8Overlay.apk=com.android.systemui.emulation.pixel_8
package:/apex/com.android.mediaprovider/priv-app/PhotopickerGoogle@360526000/PhotopickerGoogle.apk=com.google.android.photopicker
package:/product/overlay/DisplayCutoutEmulationEmu01/DisplayCutoutEmulationEmu01Overlay.apk=com.android.internal.display.cutout.emulation.emu01
package:/system/app/CertInstaller/CertInstaller.apk=com.android.certinstaller
package:/product/overlay/TeleService__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.phone.auto_generated_rro_product__
package:/product/overlay/SystemUIEmulationPixel6Pro/SystemUIEmulationPixel6ProOverlay.apk=com.android.systemui.emulation.pixel_6_pro
package:/apex/com.android.virt/priv-app/VmTerminalApp@BP22.250325.006/VmTerminalApp.apk=com.android.virtualization.terminal
package:/product/priv-app/DevicePersonalizationPrebuiltPixel2021-bfinal_aiai_20250217.00_RC08/DevicePersonalizationPrebuiltPixel2021-bfinal_aiai_20250217.00_RC08.apk=com.google.android.as
package:/system/app/SimAppDialog/SimAppDialog.apk=com.android.simappdialog
package:/product/overlay/EmulationPixel6a/EmulationPixel6aOverlay.apk=com.android.internal.emulation.pixel_6a
package:/product/app/YouTubeMusicPrebuilt/YouTubeMusicPrebuilt.apk=com.google.android.apps.youtube.music
package:/product/app/PrebuiltGoogleTelemetryTvp/PrebuiltGoogleTelemetryTvp.apk=com.google.mainline.telemetry
package:/product/overlay/EmulationPixel6/EmulationPixel6Overlay.apk=com.android.internal.emulation.pixel_6
package:/system/priv-app/DynamicSystemInstallationService/DynamicSystemInstallationService.apk=com.android.dynsystem
package:/product/overlay/EmulationPixel4/EmulationPixel4Overlay.apk=com.android.internal.emulation.pixel_4
package:/product/app/Drive/Drive.apk=com.google.android.apps.docs
package:/product/overlay/SystemUIEmulationPixel8Pro/SystemUIEmulationPixel8ProOverlay.apk=com.android.systemui.emulation.pixel_8_pro
package:/product/overlay/Traceur__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.traceur.auto_generated_rro_product__
package:/product/overlay/CarrierConfig__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.carrierconfig.auto_generated_rro_product__
package:/product/overlay/PixelConfigOverlayCommon.apk=com.google.android.overlay.pixelconfigcommon
package:/product/overlay/EmulationPixel9a/EmulationPixel9aOverlay.apk=com.android.internal.emulation.pixel_9a
package:/product/overlay/EmulationPixelFold/EmulationPixelFoldOverlay.apk=com.android.internal.emulation.pixel_fold
package:/system/priv-app/ContactsProvider/ContactsProvider.apk=com.android.providers.contacts
package:/system/priv-app/MediaProviderLegacy/MediaProviderLegacy.apk=com.android.providers.media
package:/system_ext/priv-app/GoogleServicesFramework/GoogleServicesFramework.apk=com.google.android.gsf
package:/product/overlay/EmulationPixel9ProXL/EmulationPixel9ProXLOverlay.apk=com.android.internal.emulation.pixel_9_pro_xl
package:/vendor/overlay/CarrierConfig__sdk_gphone64_x86_64__auto_generated_rro_vendor.apk=com.android.carrierconfig.auto_generated_rro_vendor__
package:/apex/com.android.appsearch/priv-app/com.google.android.appsearch.apk@360526000/com.google.android.appsearch.apk.apk=com.google.android.appsearch.apk
package:/product/overlay/EmulationPixel4a/EmulationPixel4aOverlay.apk=com.android.internal.emulation.pixel_4a
package:/product/priv-app/ConfigUpdater/ConfigUpdater.apk=com.google.android.configupdater
package:/apex/com.android.tethering/priv-app/TetheringGoogle@360526040/TetheringGoogle.apk=com.google.android.networkstack.tethering
package:/system_ext/priv-app/SettingsGoogle/SettingsGoogle.apk=com.android.settings
package:/system/app/SecureElement/SecureElement.apk=com.android.se
package:/system_ext/priv-app/MultiDisplayProvider/MultiDisplayProvider.apk=com.android.emulator.multidisplay
package:/system/app/CarrierDefaultApp/CarrierDefaultApp.apk=com.android.carrierdefaultapp
package:/product/priv-app/PartnerSetupPrebuilt/PartnerSetupPrebuilt.apk=com.google.android.partnersetup
package:/data/app/~~fUgOqJOJ6mDej7PFQKTwGA==/com.android.chrome-UXbc3qw6MSB2c4HdOVgKhQ==/Chrome.apk=com.android.chrome
package:/system/app/GooglePrintRecommendationService/GooglePrintRecommendationService.apk=com.google.android.printservice.recommendation
package:/product/app/Maps/Maps.apk=com.google.android.apps.maps
package:/system/priv-app/E2eeContactKeysProvider/E2eeContactKeysProvider.apk=com.android.providers.contactkeys
package:/product/priv-app/SafetyHubPrebuilt/SafetyHubPrebuilt.apk=com.google.android.apps.safetyhub
package:/product/app/PrebuiltGoogleAdservicesTvp/PrebuiltGoogleAdservicesTvp.apk=com.google.mainline.adservices
package:/vendor/overlay/framework-res__sdk_gphone64_x86_64__auto_generated_rro_vendor.apk=android.auto_generated_rro_vendor__
package:/system/priv-app/BackupRestoreConfirmation/BackupRestoreConfirmation.apk=com.android.backupconfirm
package:/product/overlay/SystemUIEmulationPixel9ProFold/SystemUIEmulationPixel9ProFoldOverlay.apk=com.android.systemui.emulation.pixel_9_pro_fold
package:/product/overlay/EmulationPixel3/EmulationPixel3Overlay.apk=com.android.internal.emulation.pixel_3
package:/apex/com.android.devicelock/priv-app/DeviceLockController@BP22.250325.006/DeviceLockController.apk=com.android.devicelockcontroller
package:/system_ext/priv-app/SystemUIGoogle/SystemUIGoogle.apk=com.android.systemui
package:/system/priv-app/FusedLocation/FusedLocation.apk=com.android.location.fused
package:/apex/com.android.virt/app/EmptyPayloadApp@BP22.250325.006/EmptyPayloadApp.apk=com.android.microdroid.empty_payload
package:/product/overlay/SystemUIEmulationPixel9Pro/SystemUIEmulationPixel9ProOverlay.apk=com.android.systemui.emulation.pixel_9_pro
package:/apex/com.android.bt/app/BluetoothGoogle@360526000/BluetoothGoogle.apk=com.google.android.bluetooth
package:/product/overlay/SystemUIEmulationPixel4/SystemUIEmulationPixel4Overlay.apk=com.android.systemui.emulation.pixel_4
package:/product/overlay/NavigationBarMode2Button/NavigationBarMode2ButtonOverlay.apk=com.android.internal.systemui.navbar.twobutton
package:/system_ext/priv-app/CFSatelliteService/CFSatelliteService.apk=com.google.android.telephony.satellite
package:/product/overlay/Telecom__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.server.telecom.auto_generated_rro_product__
package:/data/app/~~3C_xWLE9kUDXzsvFP08Bxg==/com.android.vending-DofoMR6flGYqx_ZDkx5etg==/base.apk=com.android.vending
package:/system/app/HTMLViewer/HTMLViewer.apk=com.android.htmlviewer
package:/product/overlay/EmulationPixel8Pro/EmulationPixel8ProOverlay.apk=com.android.internal.emulation.pixel_8_pro
package:/product/overlay/EmulatorTetheringGoogleConfigOverlay.apk=com.google.android.networkstack.tethering.emulator
package:/system/priv-app/Telecom/Telecom.apk=com.android.server.telecom
package:/system/priv-app/CellBroadcastLegacyApp/CellBroadcastLegacyApp.apk=com.android.cellbroadcastreceiver
package:/vendor/overlay/SystemUIGoogle__sdk_gphone64_x86_64__auto_generated_rro_vendor.apk=com.android.systemui.auto_generated_rro_vendor__
package:/apex/com.android.adservices/priv-app/AdServicesApkGoogle@360526040/AdServicesApkGoogle.apk=com.google.android.adservices.api
package:/product/app/SoundPickerPrebuilt/SoundPickerPrebuilt.apk=com.google.android.soundpicker
package:/product/overlay/SystemUIEmulationPixel3a/SystemUIEmulationPixel3aOverlay.apk=com.android.systemui.emulation.pixel_3a
package:/product/overlay/SystemUIEmulationPixel6/SystemUIEmulationPixel6Overlay.apk=com.android.systemui.emulation.pixel_6
package:/apex/com.android.configinfrastructure/app/DeviceConfigServiceResourcesGoogle@360526000/DeviceConfigServiceResourcesGoogle.apk=com.google.android.server.deviceconfig.resources
package:/product/overlay/NavigationBarModeGestural/NavigationBarModeGesturalOverlay.apk=com.android.internal.systemui.navbar.gestural
package:/product/overlay/GooglePermissionControllerOverlay.apk=com.google.android.overlay.permissioncontroller
package:/system/priv-app/DocumentsUIGoogle/DocumentsUIGoogle.apk=com.google.android.documentsui
package:/product/app/GoogleContacts/GoogleContacts.apk=com.google.android.contacts
package:/product/priv-app/Velvet/Velvet.apk=com.google.android.googlequicksearchbox
package:/product/app/YouTube/YouTube.apk=com.google.android.youtube
package:/product/overlay/DisplayCutoutEmulationTall/DisplayCutoutEmulationTallOverlay.apk=com.android.internal.display.cutout.emulation.tall
package:/product/overlay/SettingsGoogle__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.settings.auto_generated_rro_product__
package:/product/overlay/SimAppDialog__sdk_gphone64_x86_64__auto_generated_rro_product.apk=com.android.simappdialog.auto_generated_rro_product__
package:/product/overlay/EmulationPixel6Pro/EmulationPixel6ProOverlay.apk=com.android.internal.emulation.pixel_6_pro
package:/product/priv-app/WellbeingPrebuilt/WellbeingPrebuilt.apk=com.google.android.apps.wellbeing
package:/system/app/Traceur/Traceur.apk=com.android.traceur
package:/product/app/LatinIMEGooglePrebuilt/LatinIMEGooglePrebuilt.apk=com.google.android.inputmethod.latin
package:/vendor/overlay/TeleService__sdk_gphone64_x86_64__auto_generated_rro_vendor.apk=com.android.phone.auto_generated_rro_vendor__
package:/system/framework/framework-res.apk=android
package:/product/app/talkback/talkback.apk=com.google.android.marvin.talkback
package:/apex/com.android.extservices/priv-app/GoogleExtServices@360526000/GoogleExtServices.apk=com.google.android.ext.services
package:/product/overlay/TelephonyProvider__emulator__auto_generated_characteristics_rro.apk=com.android.providers.telephony.auto_generated_characteristics_rro
package:/system_ext/priv-app/StorageManager/StorageManager.apk=com.android.storagemanager
package:/system/priv-app/DownloadProviderUi/DownloadProviderUi.apk=com.android.providers.downloads.ui
package:/system/priv-app/VpnDialogs/VpnDialogs.apk=com.android.vpndialogs
package:/product/overlay/EmulationPixel8/EmulationPixel8Overlay.apk=com.android.internal.emulation.pixel_8
package:/data/app/~~HgwNF-LyEPF5WqNiphsCQg==/com.google.android.gms-BiXgqXLfmJwxGBiLbHWuOQ==/base.apk=com.google.android.gms
package:/system/priv-app/DeviceAsWebcam/DeviceAsWebcam.apk=com.android.DeviceAsWebcam
package:/product/overlay/SystemUIEmulationPixel3aXL/SystemUIEmulationPixel3aXLOverlay.apk=com.android.systemui.emulation.pixel_3a_xl
package:/product/overlay/SystemUIEmulationPixel5/SystemUIEmulationPixel5Overlay.apk=com.android.systemui.emulation.pixel_5
package:/product/overlay/TransparentNavigationBar/TransparentNavigationBarOverlay.apk=com.android.internal.systemui.navbar.transparent
package:/apex/com.android.permission/priv-app/GoogleSafetyCenterResources@360526020/GoogleSafetyCenterResources.apk=com.google.android.safetycenter.resources
package:/system_ext/priv-app/CarrierConfig/CarrierConfig.apk=com.android.carrierconfig
package:/product/overlay/GoogleConfigOverlay.apk=com.google.android.overlay.googleconfig
package:/product/overlay/SystemUIEmulationPixel7a/SystemUIEmulationPixel7aOverlay.apk=com.android.systemui.emulation.pixel_7a
package:/system/priv-app/CredentialManager/CredentialManager.apk=com.android.credentialmanager"#;

        let expected_list = vec![
"/apex/com.android.uwb/priv-app/ServiceUwbResourcesGoogle@360526040/ServiceUwbResourcesGoogle.apk",
"/product/priv-app/KidsSupervisionStub/KidsSupervisionStub.apk",
"/system_ext/priv-app/WallpaperPickerGoogleRelease/WallpaperPickerGoogleRelease.apk",
"/apex/com.android.cellbroadcast/priv-app/GoogleCellBroadcastServiceModule@360526020/GoogleCellBroadcastServiceModule.apk",
"/system/priv-app/TagGoogle/TagGoogle.apk",
"/system/priv-app/UserDictionaryProvider/UserDictionaryProvider.apk",
"/system/priv-app/GooglePackageInstaller/GooglePackageInstaller.apk",
"/product/priv-app/GoogleDialer/GoogleDialer.apk",
"/system/priv-app/BuiltInPrintService/BuiltInPrintService.apk",
"/apex/com.android.permission/priv-app/GooglePermissionController@360526020/GooglePermissionController.apk",
"/system/priv-app/LiveWallpapersPicker/LiveWallpapersPicker.apk",
"/system/priv-app/SoundPicker/SoundPicker.apk",
"/product/priv-app/AndroidAutoStubPrebuilt/AndroidAutoStubPrebuilt.apk",
"/apex/com.android.ondevicepersonalization/priv-app/OnDevicePersonalizationGoogle@360526000/OnDevicePersonalizationGoogle.apk",
"/system/priv-app/ExternalStorageProvider/ExternalStorageProvider.apk",
"/system/priv-app/NetworkStackGoogle/NetworkStackGoogle.apk",
"/apex/com.android.rkpd/priv-app/rkpdapp.google@360526000/rkpdapp.google.apk",
"/apex/com.android.healthfitness/priv-app/HealthConnectControllerGoogle@360526040/HealthConnectControllerGoogle.apk",
"/system_ext/priv-app/QuickAccessWallet/QuickAccessWallet.apk",
"/apex/com.android.nfcservices/priv-app/NfcNciApexMigrationGoogle@360526020/NfcNciApexMigrationGoogle.apk",
"/product/priv-app/PrebuiltBugle/PrebuiltBugle.apk",
"/system_ext/priv-app/NexusLauncherRelease/NexusLauncherRelease.apk",
"/apex/com.android.apex.cts.shim/priv-app/CtsShimPriv@MAIN/CtsShimPriv.apk",
"/apex/com.android.mediaprovider/priv-app/MediaProviderGoogle@360526000/MediaProviderGoogle.apk",
"/system/priv-app/TelephonyProvider/TelephonyProvider.apk",
"/system/priv-app/BlockedNumberProvider/BlockedNumberProvider.apk",
"/system/priv-app/CallLogBackup/CallLogBackup.apk",
"/product/priv-app/SettingsIntelligenceGooglePrebuilt/SettingsIntelligenceGooglePrebuilt.apk",
"/system_ext/priv-app/GoogleSdkSetup/GoogleSdkSetup.apk",
"/system_ext/priv-app/AvatarPickerGoogle/AvatarPickerGoogle.apk",
"/system_ext/priv-app/ThemePicker/ThemePicker.apk",
"/system/priv-app/TeleService/TeleService.apk",
"/system_ext/priv-app/EmulatorRadioConfig/EmulatorRadioConfig.apk",
"/product/priv-app/ImsServiceEntitlement/ImsServiceEntitlement.apk",
"/apex/com.android.wifi/priv-app/ServiceWifiResourcesGoogle@360526000/ServiceWifiResourcesGoogle.apk",
"/system/priv-app/IntentResolver/IntentResolver.apk",
"/system/priv-app/MusicFX/MusicFX.apk",
"/system/priv-app/MtpService/MtpService.apk",
"/system/priv-app/CalendarProvider/CalendarProvider.apk",
"/system/priv-app/SharedStorageBackup/SharedStorageBackup.apk",
"/product/priv-app/GoogleRestorePrebuilt-v717308/GoogleRestorePrebuilt-v717308.apk",
"/product/priv-app/OdadPrebuilt/OdadPrebuilt.apk",
"/apex/com.android.tethering/priv-app/ServiceConnectivityResourcesGoogle@360526040/ServiceConnectivityResourcesGoogle.apk",
"/system/priv-app/LocalTransport/LocalTransport.apk",
"/system/priv-app/InputDevices/InputDevices.apk",
"/system/priv-app/DownloadProvider/DownloadProvider.apk",
"/system_ext/priv-app/WallpaperCropper/WallpaperCropper.apk",
"/system/priv-app/DeviceDiagnostics/DeviceDiagnostics.apk",
"/system/priv-app/ONS/ONS.apk",
"/system/priv-app/ProxyHandler/ProxyHandler.apk",
"/apex/com.android.nfcservices/priv-app/NfcNciApexGoogle@360526020/NfcNciApexGoogle.apk",
"/product/priv-app/GoogleOneTimeInitializer/GoogleOneTimeInitializer.apk",
"/system/priv-app/MmsService/MmsService.apk",
"/system/priv-app/ManagedProvisioning/ManagedProvisioning.apk",
"/system/priv-app/Shell/Shell.apk",
"/system_ext/priv-app/GoogleFeedback/GoogleFeedback.apk",
"/product/priv-app/DeviceIntelligenceNetworkPrebuilt-astrea_20240329.00_RC02/DeviceIntelligenceNetworkPrebuilt-astrea_20240329.00_RC02.apk",
"/apex/com.android.cellbroadcast/priv-app/GoogleCellBroadcastApp@360526020/GoogleCellBroadcastApp.apk",
"/system/priv-app/SettingsProvider/SettingsProvider.apk",
"/apex/com.android.mediaprovider/priv-app/PhotopickerGoogle@360526000/PhotopickerGoogle.apk",
"/apex/com.android.virt/priv-app/VmTerminalApp@BP22.250325.006/VmTerminalApp.apk",
"/product/priv-app/DevicePersonalizationPrebuiltPixel2021-bfinal_aiai_20250217.00_RC08/DevicePersonalizationPrebuiltPixel2021-bfinal_aiai_20250217.00_RC08.apk",
"/system/priv-app/DynamicSystemInstallationService/DynamicSystemInstallationService.apk",
"/system/priv-app/ContactsProvider/ContactsProvider.apk",
"/system/priv-app/MediaProviderLegacy/MediaProviderLegacy.apk",
"/system_ext/priv-app/GoogleServicesFramework/GoogleServicesFramework.apk",
"/apex/com.android.appsearch/priv-app/com.google.android.appsearch.apk@360526000/com.google.android.appsearch.apk.apk",
"/product/priv-app/ConfigUpdater/ConfigUpdater.apk",
"/apex/com.android.tethering/priv-app/TetheringGoogle@360526040/TetheringGoogle.apk",
"/system_ext/priv-app/SettingsGoogle/SettingsGoogle.apk",
"/system_ext/priv-app/MultiDisplayProvider/MultiDisplayProvider.apk",
"/product/priv-app/PartnerSetupPrebuilt/PartnerSetupPrebuilt.apk",
"/system/priv-app/E2eeContactKeysProvider/E2eeContactKeysProvider.apk",
"/product/priv-app/SafetyHubPrebuilt/SafetyHubPrebuilt.apk",
"/system/priv-app/BackupRestoreConfirmation/BackupRestoreConfirmation.apk",
"/apex/com.android.devicelock/priv-app/DeviceLockController@BP22.250325.006/DeviceLockController.apk",
"/system_ext/priv-app/SystemUIGoogle/SystemUIGoogle.apk",
"/system/priv-app/FusedLocation/FusedLocation.apk",
"/system_ext/priv-app/CFSatelliteService/CFSatelliteService.apk",
"/system/priv-app/Telecom/Telecom.apk",
"/system/priv-app/CellBroadcastLegacyApp/CellBroadcastLegacyApp.apk",
"/apex/com.android.adservices/priv-app/AdServicesApkGoogle@360526040/AdServicesApkGoogle.apk",
"/system/priv-app/DocumentsUIGoogle/DocumentsUIGoogle.apk",
"/product/priv-app/Velvet/Velvet.apk",
"/product/priv-app/WellbeingPrebuilt/WellbeingPrebuilt.apk",
"/apex/com.android.extservices/priv-app/GoogleExtServices@360526000/GoogleExtServices.apk",
"/system_ext/priv-app/StorageManager/StorageManager.apk",
"/system/priv-app/DownloadProviderUi/DownloadProviderUi.apk",
"/system/priv-app/VpnDialogs/VpnDialogs.apk",
"/system/priv-app/DeviceAsWebcam/DeviceAsWebcam.apk",
"/apex/com.android.permission/priv-app/GoogleSafetyCenterResources@360526020/GoogleSafetyCenterResources.apk",
"/system_ext/priv-app/CarrierConfig/CarrierConfig.apk",
"/system/priv-app/CredentialManager/CredentialManager.apk",
];

        let mut expected_hash: HashSet<String> = HashSet::new();
        expected_hash.extend(expected_list.into_iter().map(String::from));

        mock_adb
            .expect_shell_split_streamed()
            .returning(move |_, _, on_stdout_line, _| {
                for p in emulator_output.split('\n') {
                    on_stdout_line(p).unwrap();
                }
                Ok(ExitStatus::default())
            });

        let helper = AdbDatabaseSetupHelper::new(mock_adb);
        let priv_apps = list_priv_apps(&helper).unwrap();

        assert_eq!(expected_hash, priv_apps);
    }
}
