use clap::Args;

use dtu::db::device::models::Apk;
use dtu::db::device::EMULATOR_DIFF_SOURCE;
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::{Context, DefaultContext};

use crate::printer::Printer;

#[derive(Args)]
pub struct Apks {
    /// Only show APKs that don't exist in AOSP
    ///
    /// Specifying this flag requires the emulator diff is done
    #[arg(
        short = 'n',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_non_aosp: bool,

    /// Only show privileged APKs
    #[arg(short, long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    only_priv: bool,

    /// Include the path to the APK on the device
    #[arg(short, long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    show_path: bool,
}

impl Apks {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx)?;
        meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
        let db = DeviceSqliteDatabase::new(&ctx)?;
        if self.only_non_aosp {
            meta.ensure_prereq(Prereq::EmulatorDiff)?;
            self.show_non_aosp(&ctx, &db)
        } else {
            self.show_all(&ctx, &db)
        }
    }

    fn show_non_aosp(&self, _ctx: &dyn Context, db: &dyn DeviceDatabase) -> anyhow::Result<()> {
        let diff_source = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
        let apks = db.get_apk_diffs_by_diff_id(diff_source.id)?;
        let mut filt = apks
            .iter()
            .filter(|it| !it.exists_in_diff)
            .map(|it| &it.apk);
        self.show_apks(&mut filt);
        Ok(())
    }

    fn show_all(&self, _ctx: &dyn Context, db: &dyn DeviceDatabase) -> anyhow::Result<()> {
        let apks = db.get_apks()?;
        let mut it = apks.iter();
        self.show_apks(&mut it);
        Ok(())
    }

    fn show_apks<'a>(&self, apks: &mut dyn Iterator<Item = &'a Apk>) {
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
    }
}
