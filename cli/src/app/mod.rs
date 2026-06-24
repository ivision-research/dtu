use std::borrow::Cow;
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
use dtu::app::{render_into, AppGradleBuild, AppTestStatus, TemplateRenderer, LIB_PKG_NAME};
use dtu::db::meta::db::APP_ID_KEY;
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
    Build(Build),

    /// Build and install the application
    #[command()]
    Install(Install),

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
struct Build {
    /// Application directory if not the default
    #[arg(short, long)]
    dir: Option<PathBuf>,
}

impl Build {
    fn run(self, ctx: &dyn Context) -> anyhow::Result<()> {
        println!("Building the application...");
        let meta = MetaSqliteDatabase::new(ctx)?;
        ctx.get_env("ANDROID_HOME")?;
        let app_dir = match self.dir {
            Some(v) => v,
            None => ctx.get_test_app_dir()?,
        };
        regen_templates(ctx, &meta, Some(&app_dir))?;

        let app_dir_string = app_dir.to_str().expect("valid paths");
        let gradlew = get_gradlew(&app_dir)?;
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
}

#[derive(Args)]
struct Install {
    /// Application directory if not the default
    #[arg(short, long)]
    dir: Option<PathBuf>,

    /// Don't try to automatically start the application
    #[arg(short, long)]
    no_start: bool,
}

impl Install {
    fn run(self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let app_id = meta.get_key_value(APP_ID_KEY)?;
        let adb = get_adb(ctx, true)?;
        let app_dir = match self.dir {
            Some(v) => v,
            None => ctx.get_test_app_dir()?,
        };
        let output = app_dir.join(Path::new(
            "app/build/outputs/apk/generated/app-generated.apk",
        ));
        let output_string = path_must_str(&output);
        println!("Installing the application via ADB");
        adb.install(output_string)?;
        if !self.no_start {
            println!("Starting the application...");
            am_start_app(&adb, &app_id)?;
        }
        Ok(())
    }
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
    pub fn run(self) -> anyhow::Result<()> {
        let needs_setup = match &self.command {
            Subcommand::Setup(_) => false,
            _ => true,
        };
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        if needs_setup {
            meta.ensure_prereq(Prereq::AppSetup)?;
        }
        match self.command {
            Subcommand::Setup(c) => c.run()?,
            Subcommand::ForwardServer(c) => c.run(&ctx)?,
            Subcommand::Build(c) => c.run(&ctx)?,
            Subcommand::SetAppId(c) => set_app_id(&ctx, &meta, &c.id)?,
            Subcommand::Install(c) => c.run(&ctx, &meta)?,
            Subcommand::ChangeStatus(c) => c.run(&ctx, &meta)?,
            Subcommand::RemoveTest(c) => c.run(&ctx, &meta)?,
            Subcommand::NewSystemService(c) => c.run(&ctx, &meta)?,
            Subcommand::NewProvider(c) => c.run(&ctx, &meta)?,
            Subcommand::NewGeneric(c) => c.run(&ctx, &meta)?,
            Subcommand::NewAppService(c) => c.run(&ctx, &meta)?,
            Subcommand::RunTest(c) => c.run(&ctx, &meta)?,
            Subcommand::Start => start_app(&ctx, &meta)?,
            Subcommand::StartServer => start_server(&ctx, &meta)?,
            Subcommand::ListTests => list_tests(&meta)?,
        }
        Ok(())
    }
}

pub(crate) fn regen_templates(
    ctx: &dyn Context,
    meta: &impl MetaDatabase,
    app_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let flag_file = match app_dir {
        Some(v) => Cow::Borrowed(v),
        None => Cow::Owned(ctx.get_test_app_dir()?),
    }
    .join(".dtu-noregen");
    let noregen = flag_file.exists();
    if noregen {
        return Ok(());
    }

    let app_id = meta.get_key_value(APP_ID_KEY)?;
    let template = TemplateRenderer::new(ctx, meta, &app_id);
    template.update()?;
    Ok(())
}

impl ChangeStatus {
    fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        let mut act = self.activity.clone();
        act.status = self.status;
        meta.update_app_activity(&act)?;
        regen_templates(ctx, meta, None)?;
        Ok(())
    }
}

impl RemoveTest {
    fn run(&self, ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
        meta.delete_app_activity_by_id(self.activity.id)?;
        let id = meta.get_key_value(APP_ID_KEY)?;
        let template = TemplateRenderer::new(ctx, meta, &id);
        template.update()?;
        let path = format!("app/src/main/kotlin/dtu/{}.kt", self.activity.name);
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

fn set_app_id(ctx: &dyn Context, meta: &impl MetaDatabase, id: &str) -> anyhow::Result<()> {
    meta.update_key_value(APP_ID_KEY, id)?;
    let dir = ctx.get_test_app_dir()?.join("app").join("build.gradle.kts");
    let build_template = AppGradleBuild::default().set_app_id(id);
    render_into(ctx, "build.gradle", &dir, &build_template)?;
    Ok(())
}

fn get_gradlew(app_dir: &Path) -> anyhow::Result<String> {
    let app_dir_string = path_must_str(app_dir);
    Ok(format!("{}{}gradlew", app_dir_string, OS_PATH_SEP))
}

fn start_app(ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
    let adb = get_adb(ctx, true)?;
    let app_id = meta.get_key_value(APP_ID_KEY)?;
    am_start_app(&adb, &app_id)
}

fn am_start_app(adb: &impl Adb, app_id: &str) -> anyhow::Result<()> {
    adb.shell(&format!(
        "am start-activity -n {app_id}/{LIB_PKG_NAME}.TestAppHomeActivity",
    ))?
    .err_on_status()?;
    Ok(())
}

fn start_server(ctx: &dyn Context, meta: &impl MetaDatabase) -> anyhow::Result<()> {
    let app_id = meta.get_key_value(APP_ID_KEY)?;
    let adb = get_adb(ctx, true)?;
    let cmd = format!("am start-service -n '{app_id}/{LIB_PKG_NAME}.Server'");
    let res = adb.shell(&cmd)?;
    res.err_on_status()?;
    Ok(())
}

fn list_tests(meta: &impl MetaDatabase) -> anyhow::Result<()> {
    let activities = meta.get_app_activities()?;
    for a in &activities {
        println!("{} - {}", a.name, a.status);
    }
    Ok(())
}
