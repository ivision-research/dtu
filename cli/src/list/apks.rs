use std::io;

use clap::Args;

use dtu::db::device::models::{Apk, DiffSource};
use dtu::db::{DeviceDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::{Context, DefaultContext};

use crate::diff::get_diff_source;
use crate::parsers::DiffSourceValueParser;
use crate::printer::Printer;

#[derive(Args)]
pub struct Apks {
    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
    only_new: bool,

    /// Set the diff source (only valid with -N/--only-new) otherwise the emulator is the default
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,

    /// Only show privileged APKs
    #[arg(short, long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    only_priv: bool,

    /// Include the path to the APK on the device
    #[arg(short, long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    show_path: bool,

    /// JSON output
    #[arg(short, long)]
    json: bool,
}

impl Apks {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
        let db = DeviceDatabase::new(&ctx)?;
        if self.only_new {
            self.show_new(&ctx, &meta, &db)
        } else {
            self.show_all(&ctx, &db)
        }
    }

    fn show_new(
        &self,
        ctx: &dyn Context,
        meta: &dyn MetaDatabase,
        db: &DeviceDatabase,
    ) -> anyhow::Result<()> {
        let diff_source = get_diff_source(ctx, meta, db, &self.diff_source)?;
        let apks = db.get_apk_diffs_by_diff_id(diff_source.id)?;
        let mut filt = apks
            .iter()
            .filter(|it| !it.exists_in_diff)
            .map(|it| &it.apk);
        self.show_apks(&mut filt)?;
        Ok(())
    }

    fn show_all(&self, _ctx: &dyn Context, db: &DeviceDatabase) -> anyhow::Result<()> {
        let apks = db.get_apks()?;
        let mut it = apks.iter();
        self.show_apks(&mut it)?;
        Ok(())
    }

    fn show_apks<'a>(&self, apks: &mut dyn Iterator<Item = &'a Apk>) -> anyhow::Result<()> {
        if self.json {
            serde_json::to_writer(io::stdout(), &apks.collect::<Vec<&Apk>>())?;
            return Ok(());
        }

        let printer = Printer::new();
        while let Some(apk) = apks.next() {
            if self.only_priv && !apk.is_priv {
                continue;
            }
            if self.show_path {
                printer.print(apk);
                printer.print(" - ");
                printer.println(apk.device_path.as_device_str());
            } else {
                printer.println(apk);
            }
        }

        Ok(())
    }
}
