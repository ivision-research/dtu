use std::borrow::Cow;
use std::fmt::{Display, Formatter};
use std::ops::Deref;
use std::path::PathBuf;

use diesel::prelude::*;

use dtu_proc_macro::sql_db_row;

use crate::db::graph::db::FRAMEWORK_SOURCE;
use crate::db::sql::common::{
    ApkComponent, ApkIPC, ApkIPCKind, Enablable, Exportable, Idable, PermissionMode,
    PermissionProtected,
};
use crate::manifest::{self, ApktoolManifestResolver};
use crate::utils::{path_must_str, ClassName, DevicePath};
use crate::UnknownBool;

use super::schema::*;

#[sql_db_row]
#[diesel(table_name = device_properties)]
pub struct DeviceProperty {
    pub id: i32,
    pub name: String,
    pub value: String,
}

impl Display for DeviceProperty {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:[{}]", self.name, self.value)
    }
}

#[sql_db_row]
pub struct Permission {
    pub id: i32,
    pub name: String,
    pub protection_level: String,
    pub source_apk_id: i32,
}

#[sql_db_row]
#[derive(Associations, Selectable)]
#[diesel(belongs_to(Apk))]
#[diesel(table_name = apk_permissions)]
pub struct ApkPermission {
    pub id: i32,
    pub name: String,
    pub apk_id: i32,
}

impl Display for Permission {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.name, self.protection_level)
    }
}

#[sql_db_row]
pub struct PermissionDiff {
    pub id: i32,
    pub permission: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
    pub protection_level_matches_diff: bool,
    pub diff_protection_level: Option<String>,
}

/// The result of combining an Permission with a PermissionDiff
#[derive(Clone)]
pub struct DiffedPermission {
    pub permission: Permission,
    pub exists_in_diff: bool,
    pub protection_level_matches_diff: bool,
    pub diff_protection_level: Option<String>,
}

impl Idable for DiffedPermission {
    fn get_id(&self) -> i32 {
        self.permission.id
    }
}

impl Display for DiffedPermission {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.permission)
    }
}

impl AsRef<Permission> for DiffedPermission {
    fn as_ref(&self) -> &Permission {
        &self.permission
    }
}

impl Deref for DiffedPermission {
    type Target = Permission;

    fn deref(&self) -> &Self::Target {
        &self.permission
    }
}

impl From<(Permission, PermissionDiff)> for DiffedPermission {
    fn from(value: (Permission, PermissionDiff)) -> Self {
        let (apk, diff) = value;
        Self {
            permission: apk,
            exists_in_diff: diff.exists_in_diff,
            protection_level_matches_diff: diff.protection_level_matches_diff,
            diff_protection_level: diff.diff_protection_level,
        }
    }
}

#[sql_db_row]
pub struct ProtectedBroadcast {
    pub id: i32,
    pub name: String,
}

impl Display for ProtectedBroadcast {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[sql_db_row]
pub struct UnprotectedBroadcast {
    pub id: i32,
    pub name: String,
    pub diff_source: i32,
}

impl Display for UnprotectedBroadcast {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[sql_db_row]
pub struct Apk {
    pub id: i32,
    pub app_name: String,
    pub name: String,
    pub is_debuggable: bool,
    pub is_priv: bool,
    pub device_path: DevicePath,
}

impl Apk {
    /// Get the base directory that this APK was decompiled to
    pub fn get_base_dir(&self, ctx: &dyn crate::Context) -> Option<PathBuf> {
        let dir = if self.device_path.device_file_name() == "framework-res.apk" {
            "framework"
        } else {
            "decompiled"
        };

        Some(ctx.get_apks_dir().ok()?.join(dir).join(&self.device_path))
    }

    /// Attempt to parse the APKs manifest into [a manifest::AndroidManifest]
    pub fn get_manifest(&self, ctx: &dyn crate::Context) -> Option<manifest::Manifest> {
        let path = self.get_base_dir(ctx)?.join("AndroidManifest.xml");

        log::trace!("using manifest at {}", path_must_str(&path));

        let manifest = match manifest::Manifest::from_file(&path) {
            Ok(m) => m,
            Err(e) => {
                log::error!(
                    "Failed to parse manifest file at {}: {}",
                    path_must_str(&path),
                    e
                );
                return None;
            }
        };

        Some(manifest)
    }

    /// Get an [manifest::ApktoolManifestResolver] for this APK
    pub fn get_resolver(
        &self,
        ctx: &dyn crate::Context,
    ) -> Option<manifest::ApktoolManifestResolver> {
        let path = self.get_base_dir(ctx)?;
        Some(ApktoolManifestResolver::new_from_pathbuf(path))
    }
}

/// Apk with the associated permissions that it uses
#[derive(Clone)]
pub struct ApkWithPermissions {
    pub apk: Apk,
    pub permissions: Vec<String>,
}

impl From<(Vec<ApkPermission>, Apk)> for ApkWithPermissions {
    fn from(value: (Vec<ApkPermission>, Apk)) -> Self {
        Self {
            apk: value.1,
            permissions: value.0.into_iter().map(|it| it.name).collect(),
        }
    }
}

/// The result of combining an Apk with an ApkDiff
#[derive(Clone)]
pub struct DiffedApk {
    pub apk: Apk,
    pub exists_in_diff: bool,
}

impl Idable for DiffedApk {
    fn get_id(&self) -> i32 {
        self.apk.id
    }
}

impl Display for DiffedApk {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.apk)
    }
}

impl AsRef<Apk> for DiffedApk {
    fn as_ref(&self) -> &Apk {
        &self.apk
    }
}

impl Deref for DiffedApk {
    type Target = Apk;

    fn deref(&self) -> &Self::Target {
        &self.apk
    }
}

impl From<(Apk, ApkDiff)> for DiffedApk {
    fn from(value: (Apk, ApkDiff)) -> Self {
        let (apk, diff) = value;
        Self {
            apk,
            exists_in_diff: diff.exists_in_diff,
        }
    }
}

#[sql_db_row]
pub struct ApkDiff {
    pub id: i32,
    pub apk: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
}

impl Display for Apk {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]", self.name, self.app_name)
    }
}

#[sql_db_row]
pub struct Receiver {
    pub id: i32,
    pub class_name: ClassName,
    pub permission: Option<String>,
    pub exported: bool,
    pub enabled: bool,
    pub pkg: String,
    pub apk_id: i32,
}

impl Display for Receiver {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.pkg, self.class_name)
    }
}

#[sql_db_row]
pub struct ReceiverDiff {
    pub id: i32,
    pub receiver: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
}

/// The result of combining an Receiver with an ReceiverDiff
#[derive(Clone)]
pub struct DiffedReceiver {
    pub receiver: Receiver,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
}

impl Idable for DiffedReceiver {
    fn get_id(&self) -> i32 {
        self.receiver.id
    }
}

impl Display for DiffedReceiver {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.receiver)
    }
}

impl From<(Receiver, ReceiverDiff)> for DiffedReceiver {
    fn from(value: (Receiver, ReceiverDiff)) -> Self {
        let (receiver, diff) = value;
        Self {
            receiver,
            exists_in_diff: diff.exists_in_diff,
            exported_matches_diff: diff.exported_matches_diff,
            permission_matches_diff: diff.permission_matches_diff,
            diff_permission: diff.diff_permission,
        }
    }
}

impl AsRef<Receiver> for DiffedReceiver {
    fn as_ref(&self) -> &Receiver {
        &self.receiver
    }
}

impl Deref for DiffedReceiver {
    type Target = Receiver;

    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}

#[sql_db_row]
pub struct Service {
    pub id: i32,
    pub class_name: ClassName,
    pub permission: Option<String>,
    pub exported: bool,
    pub enabled: bool,
    pub pkg: String,
    pub apk_id: i32,
    pub returns_binder: UnknownBool,
}

impl Display for Service {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.pkg, self.class_name)
    }
}

#[sql_db_row]
pub struct ServiceDiff {
    pub id: i32,
    pub service: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
}

/// The result of combining an Service with an ServiceDiff
#[derive(Clone)]
pub struct DiffedService {
    pub service: Service,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
}

impl Idable for DiffedService {
    fn get_id(&self) -> i32 {
        self.service.id
    }
}

impl Display for DiffedService {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.service)
    }
}

impl From<(Service, ServiceDiff)> for DiffedService {
    fn from(value: (Service, ServiceDiff)) -> Self {
        let (service, diff) = value;
        Self {
            service,
            exists_in_diff: diff.exists_in_diff,
            exported_matches_diff: diff.exported_matches_diff,
            permission_matches_diff: diff.permission_matches_diff,
            diff_permission: diff.diff_permission,
        }
    }
}

impl AsRef<Service> for DiffedService {
    fn as_ref(&self) -> &Service {
        &self.service
    }
}

impl Deref for DiffedService {
    type Target = Service;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

#[sql_db_row]
#[diesel(table_name = activities)]
pub struct Activity {
    pub id: i32,
    pub class_name: ClassName,
    pub permission: Option<String>,
    pub exported: bool,
    pub enabled: bool,
    pub pkg: String,
    pub apk_id: i32,
}

impl Display for Activity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.pkg, self.class_name)
    }
}

#[sql_db_row]
pub struct ActivityDiff {
    pub id: i32,
    pub activity: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
}

/// The result of combining an Activity with an ActivityDiff
#[derive(Clone)]
pub struct DiffedActivity {
    pub activity: Activity,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
}

impl Idable for DiffedActivity {
    fn get_id(&self) -> i32 {
        self.activity.id
    }
}

impl Display for DiffedActivity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.activity)
    }
}

impl From<(Activity, ActivityDiff)> for DiffedActivity {
    fn from(value: (Activity, ActivityDiff)) -> Self {
        let (activity, diff) = value;
        Self {
            activity,
            exists_in_diff: diff.exists_in_diff,
            exported_matches_diff: diff.exported_matches_diff,
            permission_matches_diff: diff.permission_matches_diff,
            diff_permission: diff.diff_permission,
        }
    }
}

impl AsRef<Activity> for DiffedActivity {
    fn as_ref(&self) -> &Activity {
        &self.activity
    }
}

impl Deref for DiffedActivity {
    type Target = Activity;

    fn deref(&self) -> &Self::Target {
        &self.activity
    }
}

macro_rules! impl_apk_ipc {
    ($name:ident) => {
        impl ApkComponent for $name {
            fn get_apk_id(&self) -> i32 {
                self.apk_id
            }
        }

        impl PermissionProtected for $name {
            fn get_generic_permission(&self) -> Option<&str> {
                self.permission.as_ref().map(|it| it.as_str())
            }
        }

        impl Exportable for $name {
            fn is_exported(&self) -> bool {
                self.exported
            }
        }

        impl Enablable for $name {
            fn is_enabled(&self) -> bool {
                self.enabled
            }
        }

        impl ApkIPC for $name {
            fn get_class_name(&self) -> ClassName {
                self.class_name.clone()
            }
            fn get_package(&self) -> Cow<'_, str> {
                Cow::Borrowed(self.pkg.as_str())
            }

            fn get_kind(&self) -> ApkIPCKind {
                ApkIPCKind::$name
            }
        }
    };
}

impl_apk_ipc!(Receiver);
impl_apk_ipc!(Activity);
impl_apk_ipc!(Service);

// TODO Eventually the schema should just have another table for authorities
//  so we can do a join

#[sql_db_row]
pub struct Provider {
    pub id: i32,
    pub name: String,
    pub authorities: String,
    pub permission: Option<String>,
    pub grant_uri_permissions: bool,
    pub read_permission: Option<String>,
    pub write_permission: Option<String>,
    pub exported: bool,
    pub enabled: bool,
    pub apk_id: i32,
}

impl ApkComponent for Provider {
    fn get_apk_id(&self) -> i32 {
        self.apk_id
    }
}

impl Exportable for Provider {
    fn is_exported(&self) -> bool {
        self.exported
    }
}

impl PermissionProtected for Provider {
    fn get_generic_permission(&self) -> Option<&str> {
        self.permission.as_ref().map(|it| it.as_str())
    }

    fn get_permission_for_mode(&self, mode: PermissionMode) -> Option<&str> {
        match mode {
            PermissionMode::Read => self.read_permission.as_ref().map(|it| it.as_str()),
            PermissionMode::Write => self.write_permission.as_ref().map(|it| it.as_str()),
            PermissionMode::Generic => self.get_generic_permission(),
        }
    }
}

impl Enablable for Provider {
    fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl ApkIPC for Provider {
    fn get_class_name(&self) -> ClassName {
        ClassName::from(&self.name)
    }

    fn get_package(&self) -> Cow<'_, str> {
        let class_name = self.get_class_name();
        Cow::Owned(class_name.pkg_as_java().to_string())
    }

    fn get_kind(&self) -> ApkIPCKind {
        ApkIPCKind::Provider
    }
}

impl Display for Provider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]", self.name, self.authorities)
    }
}

impl Provider {
    /// The [authorities] entry in the database is actually a colon separated
    /// list of authorities.
    pub fn get_authorities(&self) -> impl Iterator<Item = &str> {
        self.authorities.split(':')
    }
}

#[sql_db_row]
pub struct ProviderDiff {
    pub id: i32,
    pub provider: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
    pub write_permission_matches_diff: bool,
    pub diff_write_permission: Option<String>,
    pub read_permission_matches_diff: bool,
    pub diff_read_permission: Option<String>,
}

/// The result of combining an Provider with an ProviderDiff
#[derive(Clone)]
pub struct DiffedProvider {
    pub provider: Provider,
    pub exists_in_diff: bool,
    pub exported_matches_diff: bool,
    pub permission_matches_diff: bool,
    pub diff_permission: Option<String>,
    pub write_permission_matches_diff: bool,
    pub diff_write_permission: Option<String>,
    pub read_permission_matches_diff: bool,
    pub diff_read_permission: Option<String>,
}

impl Idable for DiffedProvider {
    fn get_id(&self) -> i32 {
        self.provider.id
    }
}

impl Display for DiffedProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.provider)
    }
}

impl From<(Provider, ProviderDiff)> for DiffedProvider {
    fn from(value: (Provider, ProviderDiff)) -> Self {
        let (provider, diff) = value;
        Self {
            provider,
            exists_in_diff: diff.exists_in_diff,
            exported_matches_diff: diff.exported_matches_diff,
            permission_matches_diff: diff.permission_matches_diff,
            diff_permission: diff.diff_permission,

            read_permission_matches_diff: diff.read_permission_matches_diff,
            diff_read_permission: diff.diff_read_permission,

            write_permission_matches_diff: diff.write_permission_matches_diff,
            diff_write_permission: diff.diff_write_permission,
        }
    }
}

impl AsRef<Provider> for DiffedProvider {
    fn as_ref(&self) -> &Provider {
        &self.provider
    }
}

impl Deref for DiffedProvider {
    type Target = Provider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

#[sql_db_row]
pub struct SystemServiceImpl {
    pub id: i32,
    pub system_service_id: i32,
    pub source: String,
    pub class_name: ClassName,
}

impl SystemServiceImpl {
    pub fn is_from_framework(&self) -> bool {
        self.source == FRAMEWORK_SOURCE
    }

    #[inline]
    pub fn is_from_apk(&self) -> bool {
        !self.is_from_framework()
    }

    pub fn apk_path(&self) -> DevicePath {
        DevicePath::from_squashed(self.source.as_str())
    }
}

impl Display for SystemServiceImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} in {}", self.class_name, self.source)
    }
}

#[sql_db_row]
pub struct SystemServiceMethod {
    pub id: i32,
    pub system_service_id: i32,
    pub transaction_id: i32,
    pub name: String,
    pub signature: Option<String>,
    pub return_type: Option<String>,
    pub smalisa_hash: Option<String>,
}

impl Display for SystemServiceMethod {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}({}) -> {}",
            self.name,
            self.get_signature(),
            self.get_return_type()
        )
    }
}

impl SystemServiceMethod {
    /// Gets the signature if there is one and returns "?" otherwise
    pub fn get_signature(&self) -> &str {
        self.signature.as_ref().map_or("", |it| it.as_str())
    }

    /// Gets the return type if there is one and returns "?" otherwise
    pub fn get_return_type(&self) -> &str {
        self.return_type.as_ref().map_or("", |it| it.as_str())
    }
}

#[sql_db_row]
pub struct SystemServiceMethodDiff {
    pub id: i32,
    pub method: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
    pub hash_matches_diff: UnknownBool,
}

/// The result of combining an SystemServiceMethod with an SystemServiceMethodDiff
#[derive(Clone)]
pub struct DiffedSystemServiceMethod {
    pub method: SystemServiceMethod,
    pub exists_in_diff: bool,
    pub hash_matches_diff: UnknownBool,
}

impl Idable for DiffedSystemServiceMethod {
    fn get_id(&self) -> i32 {
        self.method.id
    }
}

impl Display for DiffedSystemServiceMethod {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.method)
    }
}

impl From<(SystemServiceMethod, SystemServiceMethodDiff)> for DiffedSystemServiceMethod {
    fn from(value: (SystemServiceMethod, SystemServiceMethodDiff)) -> Self {
        let (method, diff) = value;
        Self {
            method,
            exists_in_diff: diff.exists_in_diff,
            hash_matches_diff: diff.hash_matches_diff,
        }
    }
}

impl AsRef<SystemServiceMethod> for DiffedSystemServiceMethod {
    fn as_ref(&self) -> &SystemServiceMethod {
        &self.method
    }
}

impl Deref for DiffedSystemServiceMethod {
    type Target = SystemServiceMethod;

    fn deref(&self) -> &Self::Target {
        &self.method
    }
}

#[sql_db_row]
pub struct SystemService {
    pub id: i32,
    pub name: String,
    pub iface: Option<ClassName>,
    pub can_get_binder: UnknownBool,
}

impl Display for SystemService {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let iface = self.iface.as_ref().map(|it| it.as_str()).unwrap_or("");
        write!(f, "{} [{}]", self.name, iface)
    }
}

impl SystemService {
    pub fn has_iface(&self) -> bool {
        self.iface.is_some()
    }
}

#[sql_db_row]
pub struct SystemServiceDiff {
    pub id: i32,
    pub system_service: i32,
    pub diff_source: i32,
    pub exists_in_diff: bool,
}

/// The result of combining an SystemService with an SystemServiceDiff
#[derive(Clone)]
pub struct DiffedSystemService {
    pub service: SystemService,
    pub exists_in_diff: bool,
}

impl Idable for DiffedSystemService {
    fn get_id(&self) -> i32 {
        self.service.id
    }
}

impl Display for DiffedSystemService {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.service)
    }
}

impl From<(SystemService, SystemServiceDiff)> for DiffedSystemService {
    fn from(value: (SystemService, SystemServiceDiff)) -> Self {
        let (service, diff) = value;
        Self {
            service,
            exists_in_diff: diff.exists_in_diff,
        }
    }
}

impl AsRef<SystemService> for DiffedSystemService {
    fn as_ref(&self) -> &SystemService {
        &self.service
    }
}

impl Deref for DiffedSystemService {
    type Target = SystemService;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

#[sql_db_row]
pub struct DiffSource {
    pub id: i32,
    pub name: String,
}

impl Display for DiffSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[sql_db_row]
pub struct FuzzResult {
    pub id: i32,
    pub service_name: String,
    pub method_name: String,
    //pub method_sig: String, TODO add this back once fast supports it
    pub exception_thrown: bool,
    pub security_exception_thrown: bool,
}
