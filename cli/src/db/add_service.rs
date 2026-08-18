use clap::{self, Args};

use dtu::db::device::diff::SystemServiceDiffTask;
use dtu::db::device::setup::{AddSystemServiceTask, ServiceMeta};
use dtu::db::device::SetupEvent;
use dtu::db::graph::{get_default_graphdb, FRAMEWORK_SOURCE};
use dtu::db::meta::get_default_metadb;
use dtu::db::{DeviceDatabase, MetaDatabase};
use dtu::prereqs::Prereq;
use dtu::tasks::{EventMonitor, NoopMonitor, TaskCanceller};
use dtu::utils::{path_must_str, ClassName, DevicePath};
use dtu::DefaultContext;

use super::get_path_for_diff_source;
use crate::parsers::DevicePathValueParser;

#[derive(Args)]
pub struct AddService {
    /// New service name
    #[arg(short, long)]
    service: String,

    /// New service interface
    #[arg(short, long)]
    iface: ClassName,

    /// Don't perform a diff
    #[arg(short, long)]
    no_diff: bool,

    /// APK containing the implementation, if applicable
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,
}

impl AddService {
    pub fn run(self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx)?;
        meta.ensure_prereq(Prereq::GraphDatabaseSetup)?;
        let db = DeviceDatabase::new(&ctx)?;
        let graph = get_default_graphdb(&ctx)?;

        let source = match self.apk.as_ref() {
            Some(v) => v.as_squashed_str(),
            None => FRAMEWORK_SOURCE,
        };

        let (_cancel, check) = TaskCanceller::new();

        let service_meta = ServiceMeta {
            service_name: self.service.clone(),
            iface: Some(self.iface.clone()),
        };

        let mut task = AddSystemServiceTask::new(
            &ctx,
            1,
            &service_meta,
            Some(&graph),
            &db,
            None::<&dyn EventMonitor<SetupEvent>>,
            &check,
        );

        task.set_source(Some(source)).set_allow_exists(false);
        task.run()?;

        if self.no_diff {
            return Ok(());
        }

        let diff_sources = db.get_diff_sources()?;

        if diff_sources.is_empty() {
            return Ok(());
        }

        let service = db.get_system_service_by_name(&self.service)?;

        println!("Diffing the new service");

        let mon = NoopMonitor::new();

        for s in &diff_sources {
            let path = get_path_for_diff_source(&ctx, &s.name)?;
            let diff_db = DeviceDatabase::new_from_path(path_must_str(&path))?;
            let task = SystemServiceDiffTask::new(s, &db, &diff_db, &service, &mon);
            task.run()?;
        }

        Ok(())
    }
}
