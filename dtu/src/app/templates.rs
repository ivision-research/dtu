use askama::{DynTemplate, Template};
use dtu_proc_macro::{define_setters, wraps_base_error};
use std::fmt;
use std::fmt::Arguments;
use std::fs::{File, OpenOptions};
use std::io;
use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};

use crate::app::AppTestStatus;
use crate::app_server::{get_server_port, APP_SERVER_PORT};
use crate::db::sql::meta::models::AppActivity;
use crate::db::sql::{self, MetaDatabase};
use crate::utils::{ensure_dir_exists, ClassName};
use crate::Context;

pub type Result<T> = std::result::Result<T, Error>;

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error rendering template {0}")]
    RenderError(String),

    #[error("{0}")]
    DB(sql::Error),

    #[error("bad zip file {0}")]
    BadZip(String),
}

impl From<sql::Error> for Error {
    fn from(value: sql::Error) -> Self {
        Self::DB(value)
    }
}

impl From<askama::Error> for Error {
    fn from(value: askama::Error) -> Self {
        Self::RenderError(value.to_string())
    }
}

macro_rules! simple_setup_template {
    ($name:ident, $path:literal) => {
        #[derive(Template)]
        #[template(path = $path)]
        struct $name<'a> {
            app_pkg: &'a str,
        }
    };
}

#[derive(Template)]
#[template(path = "app/setup/Server.kt.j2")]
struct Server<'a> {
    app_pkg: &'a str,
    app_server_port: u16,
}

impl<'a> From<&'a SetupParams<'a>> for Server<'a> {
    fn from(value: &'a SetupParams<'a>) -> Self {
        Self {
            app_pkg: value.app_pkg,
            app_server_port: value.app_server_port,
        }
    }
}

simple_setup_template!(AndroidLogger, "app/setup/AndroidLogger.kt.j2");
simple_setup_template!(AbstractLogger, "app/setup/AbstractLogger.kt.j2");
simple_setup_template!(TestService, "app/setup/TestService.kt.j2");
simple_setup_template!(AbstractTest, "app/setup/AbstractTest.kt.j2");
simple_setup_template!(IDeviceTest, "app/setup/IDeviceTest.aidl.j2");
simple_setup_template!(ILogger, "app/setup/ILogger.aidl.j2");
simple_setup_template!(AbstractBinderTest, "app/setup/AbstractBinderTest.kt.j2");
simple_setup_template!(AbstractProviderTest, "app/setup/AbstractProviderTest.kt.j2");
simple_setup_template!(AbstractServiceTest, "app/setup/AbstractServiceTest.kt.j2");
simple_setup_template!(
    AbstractSystemServiceTest,
    "app/setup/AbstractSystemServiceTest.kt.j2"
);
simple_setup_template!(AbstractTestActivity, "app/setup/AbstractTestActivity.kt.j2");
simple_setup_template!(BundleHelper, "app/setup/bundleHelper.kt.j2");
simple_setup_template!(Exceptions, "app/setup/exceptions.kt.j2");
simple_setup_template!(Extensions, "app/setup/extensions.kt.j2");
simple_setup_template!(ParcelString, "app/setup/ParcelString.kt.j2");
simple_setup_template!(LoggingBinder, "app/setup/LoggingBinder.kt.j2");
simple_setup_template!(App, "app/setup/App.kt.j2");
simple_setup_template!(GlobalConfig, "app/setup/GlobalConfig.kt.j2");
simple_setup_template!(TestAppHomeActivity, "app/setup/TestAppHomeActivity.kt.j2");
simple_setup_template!(Utils, "app/setup/Utils.kt.j2");

macro_rules! render_simple {
    ($name:ident, $base_dir:expr, $app_pkg:expr) => {
        render_simple!($name, $base_dir, $app_pkg, "kt")
    };
    ($name:ident, $base_dir:expr, $app_pkg:expr, $ext:literal) => {{
        let __tmpl = $name { app_pkg: $app_pkg };
        let __dir = $base_dir.join(concat!(stringify!($name), ".", $ext));
        let mut __writer = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&__dir)?;
        log::trace!("rendering {} to {:?}", stringify!($name), __dir);
        let mut __fwr = FileWriteAdapter::from(&mut __writer);
        __tmpl.render_into(&mut __fwr)
    }};
}

fn get_app_main_dir(ctx: &dyn Context) -> Result<PathBuf> {
    Ok(ctx.get_test_app_dir()?.join(Path::new("app/src/main")))
}

fn get_app_aidl_dir(ctx: &dyn Context, app_pkg: &str) -> Result<PathBuf> {
    let mut base = ctx.get_test_app_dir()?.join(Path::new("app/src/main/aidl"));

    for subdir in app_pkg.split('.') {
        base.push(subdir);
    }

    Ok(base)
}

fn get_app_source_dir(ctx: &dyn Context, app_pkg: &str) -> Result<PathBuf> {
    let mut base = ctx
        .get_test_app_dir()?
        .join(Path::new("app/src/main/kotlin"));

    for subdir in app_pkg.split('.') {
        base.push(subdir);
    }

    Ok(base)
}

#[derive(Template)]
#[template(path = "app/setup/settings.gradle.kts.j2")]
pub struct GradleSettings<'a> {
    pub project_name: &'a str,
}

impl<'a> From<&'a SetupParams<'a>> for GradleSettings<'a> {
    fn from(params: &'a SetupParams<'a>) -> Self {
        Self {
            project_name: params.project_name,
        }
    }
}

#[derive(Template)]
#[template(path = "app/GeneratedManifest.xml.j2")]
pub struct GeneratedManifest<'a> {
    pub app_pkg: &'a str,
    pub permissions: &'a [&'a str],
    pub activities: &'a [&'a str],
}

#[define_setters]
pub struct SetupParams<'a> {
    pub app_pkg: &'a str,

    /// Optional app_id, this will be the `applicationId` part of the
    /// app/build.gradle. If this isn't set, `app_pkg` is used.
    pub app_id: Option<&'a str>,

    pub app_server_port: u16,

    pub project_name: &'a str,

    pub build_tools_version: &'a str,
    pub compile_sdk_version: u32,
    pub min_sdk_version: u32,
    pub target_sdk_version: u32,
    pub kotlin_version: &'a str,
    pub kotlin_jvm_version: u32,
    pub android_plugin_version: &'a str,
}

struct Button<'a> {
    pub id: &'a str,
    pub target: &'a str,
    pub txt: &'a str,
}

#[derive(Template)]
#[template(path = "app/res_layout_confirmed_activity.xml.j2")]
struct ResConfirmedActivity<'a> {
    buttons: &'a [Button<'a>],
}

#[derive(Template)]
#[template(path = "app/TestAppConfirmedActivity.kt.j2")]
struct ConfirmedActivityKt<'a> {
    pub app_pkg: &'a str,
    buttons: &'a [Button<'a>],
}

#[derive(Template)]
#[template(path = "app/res_layout_failed_activity.xml.j2")]
struct ResFailedActivity<'a> {
    buttons: &'a [Button<'a>],
}

#[derive(Template)]
#[template(path = "app/TestAppFailedActivity.kt.j2")]
struct FailedActivityKt<'a> {
    pub app_pkg: &'a str,
    buttons: &'a [Button<'a>],
}

#[derive(Template)]
#[template(path = "app/res_layout_experimenting_activity.xml.j2")]
struct ResExperimentingActivity<'a> {
    buttons: &'a [Button<'a>],
}

#[derive(Template)]
#[template(path = "app/TestAppExperimentingActivity.kt.j2")]
struct ExperimentingActivityKt<'a> {
    pub app_pkg: &'a str,
    buttons: &'a [Button<'a>],
}

#[derive(Template)]
#[template(path = "app/setup/root_build.gradle.kts.j2")]
pub struct RootGradleBuild<'a> {
    pub android_plugin_version: &'a str,
    pub kotlin_version: &'a str,
}

impl<'a> From<&'a SetupParams<'a>> for RootGradleBuild<'a> {
    fn from(value: &'a SetupParams<'a>) -> Self {
        Self {
            kotlin_version: value.kotlin_version,
            android_plugin_version: value.android_plugin_version,
        }
    }
}

#[define_setters]
#[derive(Template)]
#[template(path = "app/setup/app_build.gradle.kts.j2")]
pub struct AppGradleBuild<'a> {
    pub app_id: &'a str,
    pub app_pkg: &'a str,
    pub build_tools_version: &'a str,
    pub compile_sdk_version: u32,
    pub min_sdk_version: u32,
    pub target_sdk_version: u32,
    pub kotlin_jvm_version: u32,
    pub kotlin_version: &'a str,
}

impl<'a> From<&'a SetupParams<'a>> for AppGradleBuild<'a> {
    fn from(value: &'a SetupParams<'a>) -> Self {
        Self {
            app_id: value.app_id.unwrap_or(value.app_pkg),
            app_pkg: value.app_pkg,
            build_tools_version: value.build_tools_version,
            compile_sdk_version: value.compile_sdk_version,
            min_sdk_version: value.min_sdk_version,
            target_sdk_version: value.target_sdk_version,
            kotlin_version: value.kotlin_version,
            kotlin_jvm_version: value.kotlin_jvm_version,
        }
    }
}

pub const DEFAULT_KOTLIN_JVM_VERSION: u32 = 17;
pub const DEFAULT_COMPILE_SDK: u32 = 35;
pub const DEFAULT_MIN_SDK: u32 = 27;
pub const DEFAULT_TARGET_SDK: u32 = DEFAULT_COMPILE_SDK;
pub const DEFAULT_KOTLIN_VERSION: &'static str = "2.1.10";
pub const DEFAULT_ANDROID_PLUGIN_VERSION: &'static str = "8.10.0";
pub const DEFAULT_PKG_NAME: &'static str = "c.arve";
pub const DEFAULT_APP_ID: &'static str = "c.arve";
pub const DEFAULT_PROJECT_NAME: &'static str = "DeviceTestApp";
pub const DEFAULT_BUILD_TOOLS_VERSION: &'static str = "35.0.0";

impl<'a> Default for AppGradleBuild<'a> {
    fn default() -> Self {
        Self {
            app_pkg: DEFAULT_PKG_NAME,
            app_id: DEFAULT_APP_ID,
            compile_sdk_version: DEFAULT_COMPILE_SDK,
            build_tools_version: DEFAULT_BUILD_TOOLS_VERSION,
            min_sdk_version: DEFAULT_MIN_SDK,
            target_sdk_version: DEFAULT_TARGET_SDK,
            kotlin_version: DEFAULT_KOTLIN_VERSION,
            kotlin_jvm_version: DEFAULT_KOTLIN_JVM_VERSION,
        }
    }
}

impl<'a> Default for SetupParams<'a> {
    fn default() -> Self {
        Self {
            app_pkg: DEFAULT_PKG_NAME,
            app_server_port: APP_SERVER_PORT,
            project_name: DEFAULT_PROJECT_NAME,
            app_id: Some(DEFAULT_APP_ID),
            compile_sdk_version: DEFAULT_COMPILE_SDK,
            build_tools_version: DEFAULT_BUILD_TOOLS_VERSION,
            min_sdk_version: DEFAULT_MIN_SDK,
            target_sdk_version: DEFAULT_TARGET_SDK,
            kotlin_version: DEFAULT_KOTLIN_VERSION,
            android_plugin_version: DEFAULT_ANDROID_PLUGIN_VERSION,
            kotlin_jvm_version: DEFAULT_KOTLIN_JVM_VERSION,
        }
    }
}

pub fn render_into<P: AsRef<Path> + ?Sized>(
    ctx: &dyn Context,
    name: &str,
    file: &P,
    tmpl: &dyn DynTemplate,
) -> Result<()> {
    let file = file.as_ref();

    let mut path = if file.is_absolute() {
        PathBuf::from(file)
    } else {
        ctx.get_test_app_dir()?.join(file)
    };

    if path.is_dir() {
        path = path.join(name);
    }

    if let Some(parent) = path.parent() {
        ensure_dir_exists(parent)?;
    }
    log::trace!("rendering {} into {:?}", name, path);
    let mut out = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    let mut adapter = FileWriteAdapter::from(&mut out);
    tmpl.dyn_render_into(&mut adapter)?;
    Ok(())
}

pub struct TemplateRenderer<'a> {
    app_pkg: &'a str,
    ctx: &'a dyn Context,
    meta: &'a dyn MetaDatabase,
}

impl<'a> TemplateRenderer<'a> {
    pub fn new(ctx: &'a dyn Context, meta: &'a dyn MetaDatabase, app_pkg: &'a str) -> Self {
        Self { ctx, meta, app_pkg }
    }

    #[inline]
    fn render_manifest(&self, man: GeneratedManifest) -> Result<()> {
        render_into(
            self.ctx,
            "GeneratedManifest",
            "app/src/generated/AndroidManifest.xml",
            &man,
        )
    }

    pub fn update(&self) -> Result<()> {
        let perms = self.meta.get_usable_app_permissions()?;
        let activites = self.meta.get_app_activities()?;

        let perm_names = perms
            .iter()
            .map(|it| it.permission.as_str())
            .collect::<Vec<&str>>();
        let activity_names = activites
            .iter()
            .map(|it| it.name.as_str())
            .collect::<Vec<&str>>();

        let app_server = Server {
            app_pkg: self.app_pkg,
            app_server_port: get_server_port(self.ctx)?,
        };

        let src_dir = get_app_source_dir(self.ctx, self.app_pkg)?;

        render_into(self.ctx, "Server.kt", &src_dir, &app_server)?;

        let man = GeneratedManifest {
            app_pkg: self.app_pkg,
            permissions: perm_names.as_slice(),
            activities: activity_names.as_slice(),
        };
        self.render_manifest(man)?;

        self.render_base_activities(&activites, &src_dir)?;

        Ok(())
    }

    fn render_base_activities(
        &self,
        activities: &Vec<AppActivity>,
        src_dir: &PathBuf,
    ) -> Result<()> {
        let exp_buttons = activties_to_buttons(activities, AppTestStatus::Experimenting);
        let fail_buttons = activties_to_buttons(activities, AppTestStatus::Failed);
        let conf_buttons = activties_to_buttons(activities, AppTestStatus::Confirmed);

        macro_rules! render_activity {
            ($res_name:ident, $kt_name:ident, $class:literal, $btns:expr) => {{
                let buttons = $btns;
                let __res = $res_name { buttons };

                render_into(
                    self.ctx,
                    stringify!($res_name),
                    &concat!("app/src/main/res/layout/", $class, "_activity.xml").to_lowercase(),
                    &__res,
                )?;

                let __kt = $kt_name {
                    app_pkg: self.app_pkg,
                    buttons,
                };

                let __into = src_dir.join(concat!("TestApp", $class, "Activity.kt"));

                render_into(self.ctx, stringify!($kt_name), &__into, &__kt)
            }};
        }

        render_activity!(
            ResExperimentingActivity,
            ExperimentingActivityKt,
            "Experimenting",
            exp_buttons.as_slice()
        )?;
        render_activity!(
            ResConfirmedActivity,
            ConfirmedActivityKt,
            "Confirmed",
            conf_buttons.as_slice()
        )?;
        render_activity!(
            ResFailedActivity,
            FailedActivityKt,
            "Failed",
            fail_buttons.as_slice()
        )?;
        Ok(())
    }

    /// Sets up the test application
    pub fn setup(&self, params: SetupParams) -> Result<()> {
        let app_pkg = params.app_pkg;
        if log::log_enabled!(log::Level::Trace) {
            log::trace!(
                "App base dir is {}",
                self.ctx.get_test_app_dir()?.to_str().expect("valid paths")
            );
        }
        let main_dir = get_app_main_dir(self.ctx)?;
        ensure_dir_exists(&main_dir)?;

        let settings = GradleSettings::from(&params);
        render_into(self.ctx, "GradleSettings", "settings.gradle.kts", &settings)?;

        let aidl_dir = get_app_aidl_dir(self.ctx, app_pkg)?;
        ensure_dir_exists(&aidl_dir)?;
        render_simple!(IDeviceTest, aidl_dir, app_pkg, "aidl")?;
        render_simple!(ILogger, aidl_dir, app_pkg, "aidl")?;
        let src_dir = get_app_source_dir(self.ctx, app_pkg)?;
        ensure_dir_exists(&src_dir)?;
        render_simple!(AndroidLogger, src_dir, app_pkg)?;
        render_simple!(AbstractLogger, src_dir, app_pkg)?;
        render_simple!(TestService, src_dir, app_pkg)?;
        render_simple!(AbstractTest, src_dir, app_pkg)?;
        render_simple!(AbstractBinderTest, src_dir, app_pkg)?;
        render_simple!(AbstractProviderTest, src_dir, app_pkg)?;
        render_simple!(AbstractServiceTest, src_dir, app_pkg)?;
        render_simple!(AbstractSystemServiceTest, src_dir, app_pkg)?;
        render_simple!(AbstractTestActivity, src_dir, app_pkg)?;
        render_simple!(BundleHelper, src_dir, app_pkg)?;
        render_simple!(Exceptions, src_dir, app_pkg)?;
        render_simple!(Extensions, src_dir, app_pkg)?;
        render_simple!(App, src_dir, app_pkg)?;
        render_simple!(LoggingBinder, src_dir, app_pkg)?;
        render_simple!(ParcelString, src_dir, app_pkg)?;
        render_simple!(GlobalConfig, src_dir, app_pkg)?;
        render_simple!(TestAppHomeActivity, src_dir, app_pkg)?;
        render_simple!(Utils, src_dir, app_pkg)?;

        let app_server = Server::from(&params);
        render_into(self.ctx, "Server.kt", &src_dir, &app_server)?;

        let app_build = AppGradleBuild::from(&params);
        render_into(
            self.ctx,
            "app_build.gradle.kts",
            "app/build.gradle.kts",
            &app_build,
        )?;
        let root_build = RootGradleBuild::from(&params);
        render_into(
            self.ctx,
            "root_build.grade.kts",
            "build.gradle.kts",
            &root_build,
        )?;

        let man = GeneratedManifest {
            app_pkg: params.app_pkg,
            activities: &[],
            permissions: &[],
        };
        self.render_manifest(man)?;
        self.render_base_activities(&vec![], &src_dir)?;
        self.copy_regular_files()?;
        Ok(())
    }

    fn copy_regular_files(&self) -> Result<()> {
        macro_rules! write_raw_file {
            ($name:literal) => {
                write_raw_file!($name, $name)
            };
            ($name:literal, $location:expr) => {{
                let __raw = include_bytes!(concat!("files/app/setup/", $name));
                self.write_raw_to($name, $location, __raw)
            }};
        }
        log::debug!("starting copying files");

        write_raw_file!("gitignore", ".gitignore")?;
        write_raw_file!("gradle.properties")?;
        write_raw_file!(
            "res_layout_generic_test_activity.xml",
            "app/src/main/res/layout/generic_test_activity.xml"
        )?;
        write_raw_file!(
            "res_layout_home_activity.xml",
            "app/src/main/res/layout/home_activity.xml"
        )?;
        write_raw_file!("MainManifest.xml", "app/src/main/AndroidManifest.xml")?;

        let res_zip = include_bytes!("files/app/setup/res.zip");
        self.unzip_raw_to("files/app/setup/res.zip", "app/src/main/res", res_zip)?;

        log::debug!("done copying files");

        Ok(())
    }

    fn unzip_raw_to(&self, name: &str, loc: &str, raw: &[u8]) -> Result<()> {
        let write_to = self.ctx.get_test_app_dir()?.join(loc);
        if let Some(parent) = write_to.parent() {
            ensure_dir_exists(parent)?;
        }
        log::trace!("unzipping {} to {:?}", name, write_to);
        let mut cursor = Cursor::new(raw);
        let mut archive =
            zip::ZipArchive::new(&mut cursor).map_err(|_| Error::BadZip(name.into()))?;
        for i in 0..archive.len() {
            let mut item = archive
                .by_index(i)
                .map_err(|_| Error::BadZip(name.into()))?;
            if item.is_dir() {
                continue;
            }
            let zip_path = item
                .enclosed_name()
                .ok_or_else(|| Error::BadZip(name.into()))?;
            let path = write_to.join(zip_path);
            if let Some(parent) = path.parent() {
                ensure_dir_exists(parent)?;
            }
            log::trace!("unzipping {} to {:?}", item.name(), path);
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)?;
            io::copy(&mut item, &mut file)?;
        }
        Ok(())
    }

    fn write_raw_to(&self, name: &str, loc: &str, raw: &[u8]) -> Result<()> {
        let write_to = self.ctx.get_test_app_dir()?.join(loc);
        if let Some(parent) = write_to.parent() {
            ensure_dir_exists(parent)?;
        }
        log::trace!("writing {} to {:?}", name, write_to);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&write_to)?;

        file.write_all(raw)?;
        Ok(())
    }
}

fn activties_to_buttons(activities: &Vec<AppActivity>, status: AppTestStatus) -> Vec<Button<'_>> {
    let mut btns = Vec::new();
    for a in activities {
        if a.status == status {
            btns.push(Button {
                id: a.button_android_id.as_str(),
                target: a.name.as_str(),
                txt: a.button_text.as_str(),
            })
        }
    }
    btns
}

#[derive(Template)]
#[template(path = "app/generic/TestGeneric.kt.j2")]
pub struct TestGeneric<'a> {
    /// Package name
    pub app_pkg: &'a str,
    /// Name of the class to create
    pub class: &'a str,
}

#[derive(Template)]
#[template(path = "app/service/TestServiceRaw.kt.j2")]
pub struct TestServiceRaw<'a> {
    /// Package name
    pub app_pkg: &'a str,
    /// Name of the class to create
    pub class: &'a str,
    /// Method transaction number
    pub txn_number: i32,

    /// This is the name attribute in the <service> entry. Used to retrieve
    /// an IBinder from the service.
    pub service_class: &'a ClassName,

    /// Optional package name for the service. This is only needed if the
    /// service class is not in the main package.
    pub service_pkg: Option<&'a str>,

    /// The AIDL interface for the service
    pub iface: &'a ClassName,
}

#[derive(Template)]
#[template(path = "app/service/TestServiceWithLib.kt.j2")]
pub struct TestServiceWithLib<'a> {
    /// Package name
    pub app_pkg: &'a str,
    /// Name of the class to create
    pub class: &'a str,

    /// Name of the method
    pub method: &'a str,

    /// This is the name attribute in the <service> entry. Used to retrieve
    /// an IBinder from the service.
    pub service_class: &'a ClassName,

    /// Optional package name for the service. This is only needed if the
    /// service class is not in the main package.
    pub service_pkg: Option<&'a str>,

    /// The AIDL interface for the service
    pub iface: &'a ClassName,
}

#[derive(Template)]
#[template(path = "app/provider/TestProvider.kt.j2")]
pub struct TestProvider<'a> {
    /// Package name
    pub app_pkg: &'a str,
    /// Name of the class to create
    pub class: &'a str,
    /// Provider authority
    pub authority: &'a str,
}

#[derive(Template)]
#[template(path = "app/system_service/TestSystemServiceRaw.kt.j2")]
pub struct TestSystemServiceRaw<'a> {
    /// Package name
    pub app_pkg: &'a str,
    /// Name of the class to create
    pub class: &'a str,
    /// Method transaction number
    pub txn_number: i32,
    /// Name of the target service
    pub service: &'a str,
    /// Name of the method
    pub method: &'a str,
    /// Name of the service AIDL interface
    pub iface: &'a ClassName,
}

#[derive(Template)]
#[template(path = "app/system_service/TestSystemServiceWithLib.kt.j2")]
pub struct TestSystemServiceWithLib<'a> {
    /// Package name
    pub app_pkg: &'a str,
    /// Name of the class to create
    pub class: &'a str,
    /// Name of the target service
    pub service: &'a str,
    /// Name of the method
    pub method: &'a str,
    /// Name of the service AIDL interface
    pub iface: &'a ClassName,
}

pub struct FileWriteAdapter<'a> {
    file: &'a mut File,
}

impl<'a> From<&'a mut File> for FileWriteAdapter<'a> {
    fn from(file: &'a mut File) -> Self {
        Self { file }
    }
}

impl<'a> fmt::Write for FileWriteAdapter<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.file.write_all(s.as_bytes()).map_err(|_e| fmt::Error)
    }

    fn write_fmt(self: &mut Self, args: Arguments<'_>) -> fmt::Result {
        write!(self.file, "{}", args).map_err(|_e| fmt::Error)
    }
}

mod filters {
    use crate::utils::ClassName;
    use std::fmt;

    pub fn class_pkg(class: &ClassName) -> ::askama::Result<String> {
        Ok(String::from(class.pkg_as_java()))
    }

    pub fn class_simple_name(class: &ClassName) -> ::askama::Result<String> {
        Ok(String::from(class.get_simple_class_name()))
    }

    pub fn unwrap_or<T: fmt::Display, D: fmt::Display + ?Sized>(
        opt: &Option<T>,
        default: &D,
    ) -> ::askama::Result<String> {
        Ok(opt
            .as_ref()
            .map_or_else(|| default.to_string(), |it| it.to_string()))
    }

    #[cfg(test)]
    mod test {
        use crate::utils::ClassName;
        use askama::*;

        use crate::app::templates::filters;

        macro_rules! define_test_template {
            ($name:ident, $lt:lifetime, $source:literal, { $($def:tt)* }) => {
                #[derive(Template)]
                #[template(source = $source , ext = "txt")]
                struct $name<$lt> {
                    $($def)*
                }
            };
            ($name:ident, $source:literal { $($def:tt)* }) => {
                #[derive(Template)]
                #[template(source = $source , ext = "txt")]
                struct $name {
                    $($def)*
                }
            };
        }

        macro_rules! assert_rendered {
            ($template:ident, $value:expr) => {
                assert_rendered!($template, $value, "");
            };
            ($template:ident, $value:expr, $msg:expr) => {
                assert_eq!($template.render().expect("rendering failed"), $value, $msg);
            };
        }

        #[test]
        fn test_class_simple_name() {
            define_test_template!(ClassSimpleName, 'a, "{{ class|class_simple_name }}", {
                class: &'a ClassName,
            });
            let cn = ClassName::from("Lfoo/bar/Baz$Stub;");
            let template = ClassSimpleName { class: &cn };

            assert_rendered!(template, "Baz$Stub");
        }

        #[test]
        fn test_class_pkg() {
            define_test_template!(ClassPackage, 'a, "{{ class|class_pkg }}", {
                class: &'a ClassName,
            });
            let cn = ClassName::from("foo.bar.baz.Class$Stub");
            let template = ClassPackage { class: &cn };

            assert_rendered!(template, "foo.bar.baz");
        }

        #[test]
        fn test_unwrap_or() {
            define_test_template!(UnwrapOr, 'a, "{{ class|unwrap_or(default|class_simple_name) }}", {
                class: Option<&'a str>,
                default: &'a ClassName,
            });
            let cn = ClassName::from("foo.bar.baz.Class$Stub$Proxy");
            let template = UnwrapOr {
                class: Some("Class$Stub"),
                default: &cn,
            };

            assert_rendered!(template, "Class$Stub");

            let template = UnwrapOr {
                class: None,
                default: &cn,
            };

            assert_rendered!(template, "Class$Stub$Proxy");
        }
    }
}
