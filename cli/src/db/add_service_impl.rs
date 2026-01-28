use std::path::PathBuf;

use anyhow::bail;
use clap::{self, Args};

use dtu::db::device::diff::SystemServiceDiffTask;
use dtu::db::device::models::SystemService;
use dtu::db::device::setup::{AddSystemServiceTask, ServiceMeta};
use dtu::db::device::SetupEvent;
use dtu::db::graph::{GraphDatabase, FRAMEWORK_SOURCE};
use dtu::db::DeviceDatabase;
use dtu::prereqs::Prereq;
use dtu::tasks::{EventMonitor, NoopMonitor, TaskCanceller};
use dtu::utils::{ensure_prereq, find_smali_file_for_class, path_must_str, ClassName, DevicePath};
use dtu::DefaultContext;

use super::get_path_for_diff_source;
use crate::parsers::{DevicePathValueParser, SystemServiceValueParser};

#[derive(Args)]
pub struct AddServiceImpl {
    /// Service to add the implementation for
    #[arg(short, long, value_parser = SystemServiceValueParser)]
    service: SystemService,

    /// APK containing the implementation if applicable
    #[arg(short, long, value_parser = DevicePathValueParser)]
    apk: Option<DevicePath>,

    /// The name of the implemenation class
    #[arg(short = 'C', long = "class")]
    impl_class: ClassName,

    /// The file containing the $Stub
    ///
    /// It is useful to provide this so the task doesn't need to find it
    #[arg(long)]
    stub_file: Option<PathBuf>,

    /// Don't update the diffs
    #[arg(short, long)]
    no_diff: bool,

    /// Don't remove all other implementations from the database
    #[arg(long)]
    no_remove: bool,
}

impl AddServiceImpl {
    pub fn run(&self) -> anyhow::Result<()> {
        let ctx = DefaultContext::new();
        ensure_prereq(&ctx, Prereq::PullAndDecompile)?;
        let db = DeviceDatabase::new(&ctx)?;

        let impl_path = match find_smali_file_for_class(&ctx, &self.impl_class, self.apk.as_ref()) {
            Some(v) => v,
            None => bail!("failed to find smali file for {}", self.impl_class),
        };

        let source = match self.apk.as_ref() {
            Some(v) => v.as_squashed_str(),
            None => FRAMEWORK_SOURCE,
        };

        let (_cancel, check) = TaskCanceller::new();

        if !self.no_remove {
            db.delete_system_service_impl_by_service_id(self.service.id)?;
        }

        // We have to remove all of the methods, this also removes all method
        // diffs, which we want.
        db.delete_system_service_methods_by_service_id(self.service.id)?;

        // Need to remove the system service diffing as well
        db.delete_system_service_diff_by_service_id(self.service.id)?;

        let service_meta = ServiceMeta {
            service_name: self.service.name.clone(),
            iface: self.service.iface.clone(),
        };

        let mut task = AddSystemServiceTask::new(
            &ctx,
            1,
            &service_meta,
            None::<&dyn GraphDatabase>,
            &db,
            None::<&dyn EventMonitor<SetupEvent>>,
            &check,
        );

        task.set_source(Some(source))
            .set_allow_exists(true)
            .set_impl_path(Some(&impl_path))
            .set_stub_path(self.stub_file.as_ref());

        task.run()?;

        if self.no_diff {
            return Ok(());
        }

        println!("Diffing the new service");

        let mon = NoopMonitor::new();

        let diff_sources = db.get_diff_sources()?;
        for s in &diff_sources {
            let path = get_path_for_diff_source(&ctx, &s.name)?;
            let diff_db = DeviceDatabase::new_from_path(path_must_str(&path))?;
            let task = SystemServiceDiffTask::new(s, &db, &diff_db, &self.service, &mon);
            task.run()?;
        }

        Ok(())
    }
}
