use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{self, Args};
use dtu::adb::Adb;
use dtu::prereqs::Prereq;
use dtu::utils::{path_must_str, OS_PATH_SEP};
use dtu::{Context, DefaultContext};

use crate::parsers::AppActivityValueParser;
use crate::utils::get_adb;

mod run_test;
use run_test::RunTest;

mod setup;
use setup::Setup;

mod create;
use create::*;
use dtu::app::server::get_server_port;
use dtu::app::{render_into, AppGradleBuild, AppTestStatus, TemplateRenderer};
use dtu::db::meta::db::{APP_ID_KEY, APP_PKG_KEY};
use dtu::db::meta::models::AppActivity;
use dtu::db::{MetaDatabase, MetaSqliteDatabase};

#[derive(Args)]
pub struct App {
    #[command(subcommand)]
    command: Subcommand,
}

#[derive(clap::Subcommand)]
enum Subcommand {
    /// Just build the application
    #[command()]
    Build,

    /// Build and install the application
    #[command()]
    Install,

    /// Set the application ID
    #[command()]
    SetAppId(SetAppId),

    /// Setup the base application
    #[command()]
    Setup(Setup),

    /// Create a test for a system service
    #[command()]
    NewSystemService(SystemServiceFile),

    /// Create a test for a provider
    #[command()]
    NewProvider(ProviderFile),

    /// Create a new generic test
    #[command()]
    NewGeneric(GenericFile),

    /// Create a test for a service in an application
    #[command()]
    NewAppService(ServiceFile),

    /// Change an activity's status
    #[command()]
    ChangeStatus(ChangeStatus),

    /// Remove a test completely
    #[command()]
    RemoveTest(RemoveTest),

    /// List all available tests and their status
    #[command()]
    ListTests,

    /// Use adb to forward a port to the application server
    #[command()]
    ForwardServer(ForwardServer),

    /// Use adb to ensure the application is running
    #[command()]
    Start,

    /// Use adb to ensure the server is running
    #[command()]
    StartServer,

    /// Run a given test
    #[command()]
    RunTest(RunTest),
}

#[derive(Args)]
struct SetAppId {
    /// The new app id
    #[arg(short, long)]
    id: String,
}

#[derive(Args)]
struct RemoveTest {
    /// The name of the class to remove
    #[arg(short, long, value_parser = AppActivityValueParser)]
    activity: AppActivity,
}

#[derive(Args)]
struct ChangeStatus {
    /// The name of the class to change
    #[arg(short, long, value_parser = AppActivityValueParser)]
    activity: AppActivity,

    /// The new status
    ///
    /// exp = experimenting, fail = failed, conf = confirmed
    #[arg(short, long)]
    status: AppTestStatus,
}

#[derive(Args)]
struct ForwardServer {
    /// The port on the local host
    #[arg(short = 'L', long)]
    local_port: Option<u16>,

    /// The port on the device
    #[arg(short = 'D', long)]
    device_port: Option<u16>,
}

impl App {
    pub fn run(&self) -> anyhow::Result<()> {
        let needs_setup = match &self.command {
            Subcommand::Setup(_) => false,
            _ => true,
        };
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        if needs_setup {
            meta.ensure_prereq(Prereq::AppSetup)?;
        }
        match &self.command {
            Subcommand::Setup(c) => c.run()?,
            Subcommand::ForwardServer(c) => c.run(&ctx)?,
            Subcommand::Build => self.build(&ctx)?,
            Subcommand::SetAppId(c) => self.set_app_id(&ctx, &meta, &c.id)?,
            Subcommand::Install => {
                let app_id = meta.get_key_value(APP_ID_KEY)?;
                let pkg = meta.get_key_value(APP_PKG_KEY)?;
                self.install(&ctx, &app_id, &pkg)?
            }
            Subcommand::ChangeStatus(c) => c.run(&ctx, &meta)?,
            Subcommand::RemoveTest(c) => c.run(&ctx, &meta)?,
            Subcommand::NewSystemService(c) => c.run(&ctx, &meta)?,
            Subcommand::NewProvider(c) => c.run(&ctx, &meta)?,
            Subcommand::NewGeneric(c) => c.run(&ctx, &meta)?,
            Subcommand::NewAppService(c) => c.run(&ctx, &meta)?,
            Subcommand::RunTest(c) => c.run(&ctx, &meta)?,
            Subcommand::Start => self.start_app(&ctx, &meta)?,
            Subcommand::StartServer => self.start_server(&ctx, &meta)?,
            Subcommand::ListTests => self.list_tests(&ctx, &meta)?,
        }
        Ok(())
    }

    fn start_server(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let app_pkg = meta.get_key_value(APP_PKG_KEY)?;
        let app_id = meta.get_key_value(APP_ID_KEY)?;
        let adb = get_adb(ctx, true)?;
        let cmd = format!("am start-service -n '{app_id}/{app_pkg}.Server'");
        let res = adb.shell(&cmd)?;
        res.err_on_status()?;
        Ok(())
    }

    fn list_tests(&self, _ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let activities = meta.get_app_activities()?;
        for a in &activities {
            println!("{} - {}", a.name, a.status);
        }
        Ok(())
    }

    fn start_app(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let adb = get_adb(ctx, true)?;
        let app_id = meta.get_key_value(APP_ID_KEY)?;
        let app_pkg = meta.get_key_value(APP_PKG_KEY)?;
        self.am_start_app(&adb, &app_id, &app_pkg)
    }

    fn set_app_id(
        &self,
        ctx: &dyn Context,
        meta: &impl MetaDatabase,
        id: &str,
    ) -> anyhow::Result<()> {
        let pkg = meta.get_key_value(APP_PKG_KEY)?;
        meta.update_key_value(APP_ID_KEY, id)?;
        let dir = ctx.get_test_app_dir()?.join("app").join("build.gradle.kts");
        let build_template = AppGradleBuild::default().set_app_id(id).set_app_pkg(&pkg);
        render_into(ctx, "build.gradle", &dir, &build_template)?;
        Ok(())
    }

    fn get_gradlew(&self, app_dir: &PathBuf) -> anyhow::Result<String> {
        let app_dir_string = app_dir.to_str().expect("valid paths");
        Ok(format!("{}{}gradlew", app_dir_string, OS_PATH_SEP))
    }

    fn build(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        println!("Building the application...");
        let meta = MetaSqliteDatabase::new(ctx)?;
        ctx.get_env("ANDROID_HOME")?;
        regen_templates(ctx, &meta)?;
        let app_dir = ctx.get_test_app_dir()?;
        let app_dir_string = app_dir.to_str().expect("valid paths");
        let gradlew = self.get_gradlew(&app_dir)?;
        let gradlew_cstring = CString::new(gradlew.as_str())?;
        let args = &[
            &gradlew_cstring,
            &CString::new("-p")?,
            &CString::new(app_dir_string)?,
            &CString::new("assembleGenerated")?,
        ];
        nix::unistd::execv(&gradlew_cstring, args)?;
        Ok(())
    }

    fn install(&self, ctx: &dyn Context, app_id: &str, app_pkg: &str) -> anyhow::Result<()> {
        let adb = get_adb(ctx, true)?;
        let app_dir = ctx.get_test_app_dir()?;
        let output = app_dir.join(Path::new(
            "app/build/outputs/apk/generated/app-generated.apk",
        ));
        let output_string = path_must_str(&output);
        println!("Installing the application via ADB");
        adb.install(output_string)?;
        println!("Starting the application...");
        self.am_start_app(&adb, app_id, app_pkg)
    }

    fn am_start_app(&self, adb: &impl Adb, app_id: &str, app_pkg: &str) -> anyhow::Result<()> {
        adb.shell(&format!(
            "am start-activity -n {}/{}.TestAppHomeActivity",
            app_id, app_pkg
        ))?
        .err_on_status()?;
        Ok(())
    }
}

pub(crate) fn regen_templates(ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
    let pkg = meta.get_key_value(APP_PKG_KEY)?;
    let template = TemplateRenderer::new(ctx, meta, &pkg);
    template.update()?;
    Ok(())
}

impl ChangeStatus {
    fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let mut act = self.activity.clone();
        act.status = self.status;
        meta.update_app_activity(&act)?;
        regen_templates(ctx, meta)?;
        Ok(())
    }
}

impl RemoveTest {
    fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        meta.delete_app_activity_by_id(self.activity.id)?;
        let pkg = meta.get_key_value(APP_PKG_KEY)?;
        let template = TemplateRenderer::new(ctx, meta, &pkg);
        template.update()?;
        let path = format!("app/src/main/c/arve/{}.kt", self.activity.name);
        let path = ctx.get_test_app_dir()?.join(path.as_str());
        fs::remove_file(&path)?;
        Ok(())
    }
}

impl ForwardServer {
    fn run(&self, ctx: &dyn Context) -> anyhow::Result<()> {
        let adb = get_adb(ctx, true)?;
        adb.forward_tcp_port(self.get_local_port(ctx)?, self.get_device_port(ctx)?)?;
        Ok(())
    }

    fn get_device_port(&self, ctx: &dyn Context) -> anyhow::Result<u16> {
        if let Some(p) = self.device_port {
            return Ok(p);
        }
        let port = get_server_port(ctx)?;
        Ok(port)
    }

    fn get_local_port(&self, ctx: &dyn Context) -> anyhow::Result<u16> {
        if let Some(p) = self.local_port {
            return Ok(p);
        }
        let port = get_server_port(ctx)?;
        Ok(port)
    }
}
