use clap::Args;
use dtu::db::sql::device::models::DiffedSystemService;
use dtu::db::sql::device::EMULATOR_DIFF_SOURCE;
use dtu::db::sql::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[derive(Args)]
pub struct SystemServices {
    /// Only get accessible system services
    ///
    /// Without fast results this will rely solely on whether or not an
    /// interface name was retrievable via `service list`. This indicates
    /// that the service is available _to the shell user_ but not necessarily
    /// to an application.
    #[arg(
        short = 'A',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_accessible: bool,

    /// Only get system services that aren't in AOSP
    ///
    /// Specifying this flag requires the emulator diff is done
    #[arg(
        short = 'n',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_non_aosp: bool,

    /// Only show services for which there are known implementations
    #[arg(
        short = 'I',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_impls: bool,

    /// Only show services for which there are no known implementations
    #[arg(
        short = 'N',
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_no_impls: bool,
}

impl SystemServices {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = DeviceSqliteDatabase::new(&ctx)?;
        let services = if self.only_non_aosp {
            let meta = MetaSqliteDatabase::new(&ctx)?;
            meta.ensure_prereq(Prereq::EmulatorDiff)?;
            let diff_source = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
            db.get_system_service_diffs_by_diff_id(diff_source.id)?
        } else {
            db.get_system_services()?
                .into_iter()
                .map(|it| DiffedSystemService {
                    service: it,
                    exists_in_diff: false,
                })
                .collect::<Vec<DiffedSystemService>>()
        };
        for s in services {
            if !self.include_service(&s, &db) {
                continue;
            }
            let iface = s.iface.as_ref().map(|s| s.as_str()).unwrap_or("");
            println!("{} [{}]", s.name, iface);
        }
        Ok(())
    }

    fn include_service(&self, s: &DiffedSystemService, db: &DeviceSqliteDatabase) -> bool {
        if self.only_non_aosp && s.exists_in_diff {
            return false;
        }
        if self.only_accessible
            && (s.can_get_binder.is_false() || (s.can_get_binder.is_unknown() && !s.has_iface()))
        {
            return false;
        }

        if !(self.only_impls || self.only_no_impls) {
            return true;
        }

        let impl_exists = match db.get_system_service_impls(s.id) {
            Err(e) => {
                log::warn!("finding impls for {}: {}", s.name, e);
                false
            }
            Ok(imp) => imp.len() > 0,
        };

        if self.only_impls {
            impl_exists
        } else {
            !impl_exists
        }
    }
}
