use std::io;

use clap::Args;
use dtu::db::device::models::{
    DiffSource, DiffedSystemService, SimpleSystemServiceMethod, SystemService, SystemServiceMethod,
};
use dtu::db::{DeviceDatabase, MetaSqliteDatabase};
use dtu::{Context, DefaultContext};
use itertools::Itertools;

use crate::cache_key;
use crate::diff::get_diff_source;
use crate::parsers::DiffSourceValueParser;
use crate::utils::{bool_hash_key, inum_hash_key, project_cacheable};

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

    /// Only show entries that don't exist in the given diff source (or emulator by default)
    #[arg(short = 'n', long)]
    only_new: bool,

    /// Set the diff source (only valid with -n/--only-new) otherwise the emulator is the default
    #[arg(short = 'S', long, value_parser = DiffSourceValueParser)]
    diff_source: Option<DiffSource>,

    /// JSON output
    #[arg(short, long)]
    json: bool,

    /// Also return methods, only valid with -j/--json
    #[arg(short, long)]
    with_methods: bool,

    /// Only show services for which there are known implementations
    #[arg(short = 'I', long)]
    only_impls: bool,

    /// Only show services for which there are no known implementations
    #[arg(short = 'N', long)]
    only_no_impls: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonOutput {
    service: SystemService,
    methods: Vec<SimpleSystemServiceMethod>,
}

impl SystemServices {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let db = DeviceDatabase::new(&ctx)?;

        let mut diff_id = None;

        let services = if self.only_new {
            let meta = MetaSqliteDatabase::new(&ctx)?;
            let diff_source = get_diff_source(&ctx, &meta, &db, &self.diff_source)?;
            diff_id = Some(diff_source.id);
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

        let filtered = services.into_iter().filter_map(|it| {
            if self.include_service(&it, &db) {
                Some(it.service)
            } else {
                None
            }
        });

        if self.json {
            return self.show_json(&ctx, &db, diff_id, filtered);
        }

        for s in filtered {
            let iface = s.iface.as_ref().map(|s| s.as_str()).unwrap_or("");
            println!("{} [{}]", s.name, iface);
        }
        Ok(())
    }

    fn show_json<'a, I>(
        &self,
        ctx: &dyn Context,
        db: &DeviceDatabase,
        diff_id: Option<i32>,
        services: I,
    ) -> anyhow::Result<()>
    where
        I: Iterator<Item = SystemService>,
    {
        if !self.with_methods {
            serde_json::to_writer(io::stdout(), &services.collect::<Vec<_>>())?;
            return Ok(());
        }

        let cache_path = cache_key!(
            "list-system-services-json-methods",
            bool_hash_key(self.only_accessible),
            bool_hash_key(self.only_new),
            bool_hash_key(self.only_impls),
            bool_hash_key(self.only_no_impls),
            &inum_hash_key(diff_id.unwrap_or(-1))
        );

        let json = project_cacheable(&ctx, &cache_path, false, || {
            self.get_json_output(db, diff_id, services)
        })?;

        serde_json::to_writer(io::stdout(), &json)?;
        Ok(())
    }

    fn get_json_output<I>(
        &self,
        db: &DeviceDatabase,
        diff_id: Option<i32>,
        services: I,
    ) -> anyhow::Result<Vec<JsonOutput>>
    where
        I: Iterator<Item = SystemService>,
    {
        let mut json = Vec::new();

        for svc in services {
            let methods = if let Some(id) = diff_id {
                db.get_system_service_method_diffs_for_service(svc.id, id)?
                    .into_iter()
                    .filter_map(|it| {
                        if it.exists_in_diff {
                            None
                        } else {
                            Some(it.method.into())
                        }
                    })
                    .sorted_by(|lhs: &SimpleSystemServiceMethod, rhs| lhs.txn_id.cmp(&rhs.txn_id))
                    .collect::<Vec<_>>()
            } else {
                db.get_system_service_methods_by_service_id(svc.id)?
                    .into_iter()
                    .sorted_by(|lhs: &SystemServiceMethod, rhs| {
                        lhs.transaction_id.cmp(&rhs.transaction_id)
                    })
                    .map(SimpleSystemServiceMethod::from)
                    .collect::<Vec<_>>()
            };

            json.push(JsonOutput {
                service: svc,
                methods,
            })
        }

        Ok(json)
    }

    fn include_service(&self, s: &DiffedSystemService, db: &DeviceDatabase) -> bool {
        if self.only_new && s.exists_in_diff {
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
