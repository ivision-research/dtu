use std::borrow::Cow;
use std::collections::HashMap;
use std::io::stdout;

use clap::Args;
use dtu::db::device::models::{DiffedSystemServiceMethod, SystemService, SystemServiceMethod};
use dtu::db::device::EMULATOR_DIFF_SOURCE;
use dtu::db::meta::get_default_metadb;
use dtu::db::{DeviceDatabase, MetaDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;
use serde::Serialize;

use crate::parsers::SystemServiceValueParser;

#[derive(Args)]
pub struct SystemServiceMethods {
    /// The service to get the methods for, otherwise get them all
    #[arg(short, long, value_parser = SystemServiceValueParser)]
    service: Option<SystemService>,

    /// Only get system services that aren't in AOSP
    ///
    /// Specifying this flag requires the emulator diff is done
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        default_value_t = false,
    )]
    only_non_aosp: bool,

    /// Print the results as JSON
    #[arg(short, long)]
    json: bool,
}

#[derive(Serialize)]
struct MethodDef {
    txn: i32,
    name: String,
    sig: Cow<'static, str>,
    ret: Cow<'static, str>,
}

#[derive(Serialize)]
struct ServiceMethods {
    service: String,
    methods: Vec<MethodDef>,
}

impl From<DiffedSystemServiceMethod> for MethodDef {
    fn from(value: DiffedSystemServiceMethod) -> Self {
        let method = value.method;
        Self::from(method)
    }
}

impl From<SystemServiceMethod> for MethodDef {
    fn from(value: SystemServiceMethod) -> Self {
        let sig = match value.signature {
            Some(s) => Cow::Owned(s),
            None => Cow::Borrowed(""),
        };
        let ret = match value.return_type {
            Some(s) => Cow::Owned(s),
            None => Cow::Borrowed(""),
        };
        let txn = value.transaction_id;
        let name = value.name;

        Self {
            txn,
            name,
            sig,
            ret,
        }
    }
}

type Methods = Vec<ServiceMethods>;

impl SystemServiceMethods {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx)?;
        meta.ensure_prereq(Prereq::SQLDatabaseSetup)?;
        if self.only_non_aosp {
            meta.ensure_prereq(Prereq::EmulatorDiff)?;
        }

        let db = DeviceDatabase::new(&ctx)?;

        let methods = self.get_map(&db)?;

        if self.json {
            serde_json::to_writer(stdout(), &methods)?;
            return Ok(());
        }

        let need_name = self.service.is_none();

        for mut e in methods {
            if need_name {
                println!("{}", e.service);
            }

            e.methods.sort_by_key(|it| it.txn);

            for m in e.methods {
                if need_name {
                    print!("   ");
                }
                println!("{} - {}({}): {}", m.txn, m.name, m.sig, m.ret);
            }
        }

        Ok(())
    }

    fn get_map(&self, db: &DeviceDatabase) -> anyhow::Result<Methods> {
        match &self.service {
            Some(s) => self.get_methods_for_service(db, s),
            None => self.get_all_methods(db),
        }
    }

    fn get_methods_for_service(
        &self,
        db: &DeviceDatabase,
        service: &SystemService,
    ) -> anyhow::Result<Methods> {
        let all_methods = if self.only_non_aosp {
            let diff = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
            let methods = db.get_system_service_method_diffs_for_service(service.id, diff.id)?;
            methods
                .into_iter()
                .filter_map(|it| {
                    if it.exists_in_diff {
                        Some(MethodDef::from(it))
                    } else {
                        None
                    }
                })
                .collect::<Vec<MethodDef>>()
        } else {
            db.get_system_service_methods_by_service_id(service.id)?
                .into_iter()
                .map(MethodDef::from)
                .collect::<Vec<MethodDef>>()
        };
        Ok(vec![ServiceMethods {
            service: service.name.clone(),
            methods: all_methods,
        }])
    }

    fn get_all_methods(&self, db: &DeviceDatabase) -> anyhow::Result<Methods> {
        let mut map: HashMap<String, Vec<MethodDef>> = HashMap::new();

        if self.only_non_aosp {
            let diff_source = db.get_diff_source_by_name(EMULATOR_DIFF_SOURCE)?;
            let methods = db
                .get_system_service_method_diffs_by_diff_id(diff_source.id)?
                .into_iter()
                .filter(|it| it.exists_in_diff);

            Self::collect_methods(db, &mut map, methods, |m| m.system_service_id)?;
        } else {
            let methods = db.get_system_service_methods()?;
            Self::collect_methods(db, &mut map, methods, |m| m.system_service_id)?;
        };

        Ok(map
            .into_iter()
            .map(|(service, methods)| ServiceMethods { service, methods })
            .collect::<Vec<ServiceMethods>>())
    }

    fn collect_methods<T, It, GetServiceId>(
        db: &DeviceDatabase,
        map: &mut HashMap<String, Vec<MethodDef>>,
        methods: It,
        get_service_id: GetServiceId,
    ) -> anyhow::Result<()>
    where
        T: Into<MethodDef>,
        It: IntoIterator<Item = T>,
        GetServiceId: Fn(&T) -> i32,
    {
        let system_services = db.get_system_services()?;
        let mut service_lookup = HashMap::new();
        for s in system_services {
            service_lookup.insert(s.id, s.name);
        }
        for m in methods {
            let service_id = get_service_id(&m);
            let service = match service_lookup.get(&service_id) {
                None => continue,
                Some(s) => s,
            };
            match map.get_mut(service) {
                Some(v) => v.push(m.into()),
                None => {
                    let v = vec![m.into()];
                    map.insert(service.clone(), v);
                }
            }
        }
        Ok(())
    }
}
