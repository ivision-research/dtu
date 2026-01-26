use std::borrow::Cow;
use std::fs;

use crate::utils::get_adb;
use anyhow::bail;
use clap::{self, Args};
use dtu::adb::Adb;
use dtu::app::{SetupParams, TemplateRenderer, DEFAULT_APP_ID, DEFAULT_PKG_NAME};
use dtu::db::meta::db::{APP_ID_KEY, APP_PKG_KEY};
use dtu::db::meta::models::InsertAppPermission;
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::ensure_dir_exists;
use dtu::{run_cmd, Context, DefaultContext};

#[derive(Args)]
pub struct Setup {
    /// Force the test app to be recreated if it exists
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    force: bool,

    /// Set the gradle version
    #[arg(long, default_value_t = String::from("9.3"))]
    gradle_version: String,

    /// Set the project name, otherwise generated from the device
    #[arg(short, long)]
    project_name: Option<String>,

    /// Set the application package
    #[arg(short = 'P', long, default_value_t = String::from(DEFAULT_PKG_NAME))]
    pkg: String,

    /// Set the application id
    #[arg(short = 'I', long, default_value_t = String::from(DEFAULT_APP_ID))]
    app_id: String,
}

impl Setup {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = MetaSqliteDatabase::new(&ctx)?;
        db.ensure_prereq(Prereq::SQLDatabaseSetup)?;

        let app_dir = ctx.get_test_app_dir()?;

        if db.prereq_done(Prereq::AppSetup)? {
            if !self.force {
                bail!("test app already set up");
            }
            db.wipe_app_data()?;
            if app_dir.exists() {
                fs::remove_dir_all(&app_dir)?;
            }
        }

        db.update_key_value(APP_ID_KEY, &self.app_id)?;
        db.update_key_value(APP_PKG_KEY, &self.pkg)?;

        ensure_dir_exists(&app_dir)?;

        let project_name = self.get_project_name(&ctx);

        log::trace!("initializing gradle");
        self.init_gradle(&ctx, &self.gradle_version, &project_name)?;

        log::trace!("setting up meta database");
        let device_db = DeviceSqliteDatabase::new(&ctx)?;
        let perms = device_db.get_normal_permissions()?;
        let app_perms = perms
            .iter()
            .map(|it| InsertAppPermission {
                permission: it.name.as_str(),
                usable: true,
            })
            .collect::<Vec<InsertAppPermission>>();
        db.add_app_permissions(app_perms.as_slice())?;

        log::trace!("writing templates");
        let templates = TemplateRenderer::new(&ctx, &db, &self.pkg);

        let setup_params = SetupParams::default()
            .set_project_name(project_name.as_ref())
            .set_app_id(Some(self.app_id.as_str()))
            .set_app_pkg(&self.pkg);

        templates.setup(setup_params)?;

        db.update_prereq(Prereq::AppSetup, true)?;

        Ok(())
    }

    fn init_gradle(
        &self,
        ctx: &dyn Context,
        version: &str,
        project_name: &str,
    ) -> anyhow::Result<()> {
        let cmd = ctx.get_bin("gradle")?;
        let app_dir = ctx.get_test_app_dir()?;
        let app_dir_string = app_dir.to_str().expect("valid paths");
        ensure_dir_exists(&app_dir)?;
        let args = &[
            "init",
            "-p",
            app_dir_string,
            "--type",
            "basic",
            "--dsl",
            "kotlin",
            "--project-name",
            project_name,
        ];
        run_cmd(cmd.as_str(), args)?.err_on_status()?;
        run_cmd(
            cmd.as_str(),
            &["-p", app_dir_string, "wrapper", "--gradle-version", version],
        )?
        .err_on_status()?;
        Ok(())
    }

    fn get_project_name(&self, ctx: &dyn Context) -> Cow<'_, str> {
        if let Some(s) = self.project_name.as_ref() {
            return Cow::Borrowed(s.as_str());
        }

        let default = String::from("DeviceTestApp");
        let adb = match get_adb(&ctx, true) {
            Ok(it) => it,
            Err(_) => return Cow::Owned(default),
        };

        let model = self.get_adb_prop(&adb, "ro.product.model");
        match model {
            None => Cow::Owned(default),
            Some(model) => {
                let manufacturer = self.get_adb_prop(&adb, "ro.product.manufacturer");
                match manufacturer {
                    None => Cow::Owned(model),
                    Some(prefix) => Cow::Owned(format!("{}-{}", prefix, model)),
                }
            }
        }
    }

    fn get_adb_prop(&self, adb: &dyn Adb, prop: &str) -> Option<String> {
        let cmd = format!("getprop {}", prop);
        match adb.shell(&cmd) {
            Err(e) => {
                log::warn!("failed to get prop {}: {}", prop, e);
                None
            }
            Ok(it) => match it.err_on_status() {
                Ok(v) => Some(v.stdout_utf8_lossy().trim().to_string()),
                Err(e) => {
                    log::warn!("failed to get prop {}: {}", prop, e);
                    None
                }
            },
        }
    }
}
