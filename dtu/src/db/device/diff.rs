use std::collections::{HashMap, HashSet};

use dtu_proc_macro::wraps_base_error;

use crate::db::device::db::DeviceDatabase;
use crate::db::device::models::*;
use crate::db::device::models::{Apk, DiffSource, SystemService, SystemServiceMethod};
use crate::db::{self, ApkIPC, Error, Idable, PermissionMode};
use crate::tasks::{EventMonitor, TaskCancelCheck};
use crate::utils::ClassName;
use crate::UnknownBool;

/// Events fired by the DiffTask
pub enum DiffEvent {
    SystemServicesStarted {
        count: usize,
    },
    SystemService {
        id: i32,
        name: String,
        exists: bool,
    },
    SystemServicesEnded,

    ApksStarted {
        count: usize,
    },
    Apk {
        id: i32,
        name: String,
        exists: bool,
    },
    ApksEnded,

    ReceiversStarted {
        count: usize,
    },
    Receiver {
        id: i32,
        class: ClassName,
        exists: bool,
    },
    ReceiversEnded,

    ServicesStarted {
        count: usize,
    },
    Service {
        id: i32,
        class: ClassName,
        exists: bool,
    },
    ServicesEnded,

    ActivitiesStarted {
        count: usize,
    },
    Activity {
        id: i32,
        class: ClassName,
        exists: bool,
    },
    ActivitiesEnded,

    ProvidersStarted {
        count: usize,
    },
    Provider {
        id: i32,
        authorities: String,
        exists: bool,
    },
    ProvidersEnded,

    PermissionsStarted {
        count: usize,
    },
    Permission {
        id: i32,
        permission: String,
        matches: bool,
        exists: bool,
    },
    PermissionsEnded,
}

type Evt = DiffEvent;

pub struct DiffOptions {
    source: DiffSource,
}

impl DiffOptions {
    pub fn new(source: DiffSource) -> Self {
        Self { source }
    }
}

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("user cancelled")]
    Cancelled,
    #[error("database error {0}")]
    DB(Error),
}

impl From<Error> for DiffError {
    fn from(value: Error) -> Self {
        Self::DB(value)
    }
}

pub type DiffResult<T> = Result<T, DiffError>;

// Task to just diff a single system service
pub struct SystemServiceDiffTask<'a> {
    db: &'a DeviceDatabase,
    diff_db: &'a DeviceDatabase,
    source: &'a DiffSource,
    system_service: &'a SystemService,
    monitor: &'a dyn EventMonitor<DiffEvent>,
}

impl<'a> SystemServiceDiffTask<'a> {
    pub fn new(
        source: &'a DiffSource,
        db: &'a DeviceDatabase,
        diff_db: &'a DeviceDatabase,
        system_service: &'a SystemService,
        monitor: &'a dyn EventMonitor<DiffEvent>,
    ) -> Self {
        Self {
            source,
            db,
            diff_db,
            system_service,
            monitor,
        }
    }

    /// Entrypoint for the SystemServiceDiffTask
    pub fn run(&self) -> DiffResult<()> {
        // Check if the system service exists in the diff
        match self
            .diff_db
            .get_system_service_by_name(&self.system_service.name)
        {
            // If it doesn't, this is a new system service with respect to the diff
            Err(Error::NotFound) => self.do_new_system_service(),
            // Otherwise, go a bit deeper
            Ok(v) => self.do_system_service_diff(&v),
            Err(e) => Err(e.into()),
        }
    }

    /// Diff a system service that exists in the diff database
    fn do_system_service_diff(&self, diff: &SystemService) -> DiffResult<()> {
        let diff_entry = InsertSystemServiceDiff::new(self.system_service.id, self.source.id, true);
        self.db.add_system_service_diff(&diff_entry)?;

        let device_methods = self
            .db
            .get_system_service_methods_by_service_id(self.system_service.id)?;

        let diff_methods = self
            .diff_db
            .get_system_service_methods_by_service_id(diff.id)?;
        self.do_service_methods_diff(device_methods, diff_methods)?;
        self.trigger(&diff_entry);
        Ok(())
    }

    fn update_service_methods_not_in_diff(
        &self,
        device_methods: Vec<SystemServiceMethod>,
    ) -> DiffResult<()> {
        for m in device_methods.iter() {
            self.add_method_not_in_diff(m)?;
        }
        Ok(())
    }

    fn add_method_not_in_diff(&self, m: &SystemServiceMethod) -> DiffResult<()> {
        let ins = InsertSystemServiceMethodDiff {
            method: m.id,
            diff_source: self.source.id,
            exists_in_diff: false,
            hash_matches_diff: UnknownBool::False,
        };
        self.db.add_system_service_method_diff(&ins)?;
        Ok(())
    }

    fn do_service_methods_diff(
        &self,
        device_methods: Vec<SystemServiceMethod>,
        aosp_methods: Vec<SystemServiceMethod>,
    ) -> DiffResult<()> {
        let get_method_def = |m: &SystemServiceMethod| {
            format!(
                "{}{}{}",
                m.name,
                m.signature.as_ref().map(|s| s.as_str()).unwrap_or(""),
                m.return_type.as_ref().map(|s| s.as_str()).unwrap_or("")
            )
        };

        let mut diff_methods_hash = HashMap::new();
        diff_methods_hash.extend(aosp_methods.into_iter().map(|m| (get_method_def(&m), m)));

        for m in device_methods.iter() {
            let search = get_method_def(m);
            let diff = diff_methods_hash.get(&search);
            self.update_method_diff(m, diff)?;
        }

        Ok(())
    }

    fn update_method_diff(
        &self,
        device_method: &SystemServiceMethod,
        diff_method: Option<&SystemServiceMethod>,
    ) -> DiffResult<()> {
        let diff_method = match diff_method {
            None => return self.add_method_not_in_diff(device_method),
            Some(m) => m,
        };

        let hash_matches = match device_method.smalisa_hash.as_ref() {
            Some(dev) => match diff_method.signature.as_ref() {
                Some(diff) => UnknownBool::from(dev == diff),
                None => UnknownBool::Unknown,
            },
            None => UnknownBool::Unknown,
        };

        let ins = InsertSystemServiceMethodDiff {
            method: device_method.id,
            diff_source: self.source.id,
            exists_in_diff: true,
            hash_matches_diff: hash_matches,
        };

        self.db.add_system_service_method_diff(&ins)?;
        Ok(())
    }

    /// Add a system service that doesn't exist in the diff
    fn do_new_system_service(&self) -> DiffResult<()> {
        let diff_entry =
            InsertSystemServiceDiff::new(self.system_service.id, self.source.id, false);
        self.trigger(&diff_entry);
        self.db.add_system_service_diff(&diff_entry)?;

        let device_methods = self
            .db
            .get_system_service_methods_by_service_id(self.system_service.id)?;
        self.update_service_methods_not_in_diff(device_methods)
    }

    fn trigger(&self, diff: &InsertSystemServiceDiff) {
        let evt = Evt::SystemService {
            id: self.system_service.id,
            name: self.system_service.name.clone(),
            exists: diff.exists_in_diff,
        };
        self.monitor.on_event(evt);
    }
}

// Task to run a full device database diff
pub struct DiffTask<'a> {
    db: &'a DeviceDatabase,
    diff_db: &'a DeviceDatabase,
    monitor: &'a dyn EventMonitor<DiffEvent>,
    cancel: TaskCancelCheck,
    source: DiffSource,
    pub do_system_services: bool,
    pub do_apks: bool,
}

impl<'a> DiffTask<'a> {
    pub fn new(
        opts: DiffOptions,
        db: &'a DeviceDatabase,
        diff_db: &'a DeviceDatabase,
        cancel: TaskCancelCheck,
        monitor: &'a dyn EventMonitor<DiffEvent>,
    ) -> Self {
        Self {
            source: opts.source,
            cancel,
            db,
            diff_db,
            monitor,
            do_apks: true,
            do_system_services: true,
        }
    }

    pub fn run(&self) -> DiffResult<()> {
        self.cancel_check()?;
        log::debug!("diffing system services");
        if self.do_system_services {
            self.diff_system_services()?;
        }

        if !self.do_apks {
            return Ok(());
        }

        self.cancel_check()?;
        log::debug!("diffing apks");
        self.diff_apks()?;
        self.cancel_check()?;
        log::debug!("diffing receivers");
        self.diff_receivers()?;
        self.cancel_check()?;
        log::debug!("diffing activities");
        self.diff_activities()?;
        self.cancel_check()?;
        log::debug!("diffing services");
        self.diff_services()?;
        self.cancel_check()?;
        log::debug!("diffing providers");
        self.diff_providers()?;
        self.cancel_check()?;
        log::debug!("diffing permissions");
        self.diff_permissions()?;
        Ok(())
    }

    #[inline]
    fn cancel_check(&self) -> DiffResult<()> {
        self.cancel.check(DiffError::Cancelled)
    }

    fn diff_system_services(&self) -> DiffResult<()> {
        let system_services = self.db.get_system_services()?;
        self.trigger(Evt::SystemServicesStarted {
            count: system_services.len(),
        });
        for s in system_services {
            let task =
                SystemServiceDiffTask::new(&self.source, self.db, self.diff_db, &s, self.monitor);
            task.run()?;
            self.cancel_check()?;
        }
        self.trigger(Evt::SystemServicesEnded);
        Ok(())
    }

    /// Generic way to get all of the given type that still needs diffing
    ///
    /// `get_all` should return all of the given type (ie get_apks). `get_done` should return all
    /// of the given type for the given diff id (ie get_apk_diffs_by_diff_id)
    ///
    /// This allows rerunning this task without redoing the entire diff
    fn get_diffable<T, D, GetAll, GetDone>(
        &self,
        get_all: GetAll,
        get_done: GetDone,
    ) -> DiffResult<Vec<T>>
    where
        T: Idable,
        D: Idable,
        GetAll: Fn(&DeviceDatabase) -> db::Result<Vec<T>>,
        GetDone: Fn(&DeviceDatabase, i32) -> db::Result<Vec<D>>,
    {
        let all = get_all(self.db)?;
        let done = get_done(self.db, self.source.id)?
            .into_iter()
            .map(|it| it.get_id())
            .collect::<HashSet<i32>>();

        let filtered = all
            .into_iter()
            .filter(|it| !done.contains(&it.get_id()))
            .collect::<Vec<T>>();
        Ok(filtered)
    }

    fn get_diffable_apks(&self) -> DiffResult<Vec<Apk>> {
        self.get_diffable(|db| db.get_apks(), |db, id| db.get_apk_diffs_by_diff_id(id))
    }

    fn get_diffable_receivers(&self) -> DiffResult<Vec<Receiver>> {
        self.get_diffable(
            |db| db.get_receivers(),
            |db, id| db.get_receiver_diffs_by_diff_id(id),
        )
    }

    fn get_diffable_services(&self) -> DiffResult<Vec<Service>> {
        self.get_diffable(
            |db| db.get_services(),
            |db, id| db.get_service_diffs_by_diff_id(id),
        )
    }

    fn get_diffable_activities(&self) -> DiffResult<Vec<Activity>> {
        self.get_diffable(
            |db| db.get_activities(),
            |db, id| db.get_activity_diffs_by_diff_id(id),
        )
    }

    fn get_diffable_providers(&self) -> DiffResult<Vec<Provider>> {
        self.get_diffable(
            |db| db.get_providers(),
            |db, id| db.get_provider_diffs_by_diff_id(id),
        )
    }

    fn get_diffable_permissions(&self) -> DiffResult<Vec<Permission>> {
        self.get_diffable(
            |db| db.get_permissions(),
            |db, id| db.get_permission_diffs_by_diff_id(id),
        )
    }

    fn diff_apks(&self) -> DiffResult<()> {
        let device = self.get_diffable_apks()?;
        let diff_lst = self.diff_db.get_apks()?;
        let mut diff = HashMap::new();
        diff.extend(diff_lst.into_iter().map(|it| (it.name.clone(), it)));

        self.trigger(Evt::ApksStarted {
            count: device.len(),
        });

        for apk in device.iter() {
            let diff_apk = diff.get(&apk.name);
            self.do_apk_diff(apk, diff_apk)?;
        }

        self.trigger(Evt::ApksEnded);

        Ok(())
    }

    fn do_apk_diff(&self, device: &Apk, diff: Option<&Apk>) -> DiffResult<()> {
        let ins = InsertApkDiff::new(device.id, self.source.id, diff.is_some());
        self.db.add_apk_diff(&ins)?;
        let evt = Evt::Apk {
            id: device.id,
            name: device.name.to_string(),
            exists: diff.is_some(),
        };
        self.trigger(evt);
        Ok(())
    }

    fn diff_receivers(&self) -> DiffResult<()> {
        let device = self.get_diffable_receivers()?;
        let diff = self.diff_db.get_receivers()?;
        let src = self.source.id;
        self.trigger(Evt::ReceiversStarted {
            count: device.len(),
        });
        self.diff_apk_ipc(device, diff, |rcv, db, id, exists, exported, permission| {
            let ins = InsertReceiverDiff::new(id, src, exists, exported, permission);
            db.add_receiver_diff(&ins)?;
            let evt = Evt::Receiver {
                id,
                exists,
                class: rcv.class_name.clone(),
            };
            self.trigger(evt);
            Ok(())
        })?;
        self.trigger(Evt::ReceiversEnded);
        Ok(())
    }

    fn diff_services(&self) -> DiffResult<()> {
        let device = self.get_diffable_services()?;
        let diff = self.diff_db.get_services()?;
        let src = self.source.id;
        self.trigger(Evt::ServicesStarted {
            count: device.len(),
        });
        self.diff_apk_ipc(device, diff, |svc, db, id, exists, exported, permission| {
            let ins = InsertServiceDiff::new(id, src, exists, exported, permission);
            db.add_service_diff(&ins)?;
            let evt = Evt::Service {
                id,
                exists,
                class: svc.class_name.clone(),
            };
            self.trigger(evt);
            Ok(())
        })?;
        self.trigger(Evt::ServicesEnded);
        Ok(())
    }

    fn diff_activities(&self) -> DiffResult<()> {
        let device = self.get_diffable_activities()?;
        let diff = self.diff_db.get_activities()?;
        let src = self.source.id;
        self.trigger(Evt::ActivitiesStarted {
            count: device.len(),
        });

        self.diff_apk_ipc(device, diff, |act, db, id, exists, exported, permission| {
            let ins = InsertActivityDiff::new(id, src, exists, exported, permission);
            db.add_activity_diff(&ins)?;
            let evt = Evt::Activity {
                id,
                exists,
                class: act.class_name.clone(),
            };
            self.trigger(evt);

            Ok(())
        })?;
        self.trigger(Evt::ActivitiesEnded);

        Ok(())
    }

    fn diff_apk_ipc<T, F>(&self, device: Vec<T>, diff: Vec<T>, insert: F) -> DiffResult<()>
    where
        T: ApkIPC,
        F: Fn(&T, &DeviceDatabase, i32, bool, bool, bool) -> DiffResult<()>,
    {
        let mut diff_map: HashMap<ClassName, T> = HashMap::new();
        diff_map.extend(diff.into_iter().map(|it| (it.get_class_name().clone(), it)));

        for d in device.iter() {
            self.do_apk_ipc_diff(d, diff_map.get(&d.get_class_name()), &insert)?
        }

        Ok(())
    }
    fn do_apk_ipc_diff<T, F>(&self, device: &T, diff: Option<&T>, insert: &F) -> DiffResult<()>
    where
        T: ApkIPC,
        F: Fn(&T, &DeviceDatabase, i32, bool, bool, bool) -> DiffResult<()>,
    {
        let id = device.get_id();
        let diff = match diff {
            None => return insert(device, self.db, id, false, false, false),
            Some(v) => v,
        };

        let perms_match = if !device.requires_permission() {
            !diff.requires_permission()
        } else {
            let modes = &[
                PermissionMode::Read,
                PermissionMode::Write,
                PermissionMode::Generic,
            ];

            modes
                .iter()
                .all(|it| device.get_permission_for_mode(*it) == diff.get_permission_for_mode(*it))
        };

        let exported_matches = if device.is_exported() {
            diff.is_exported()
        } else {
            !diff.is_exported()
        };

        insert(device, self.db, id, true, exported_matches, perms_match)
    }

    fn diff_permissions(&self) -> DiffResult<()> {
        let device_perms = self.get_diffable_permissions()?;
        let diff_perms_lst = self.diff_db.get_permissions()?;
        self.trigger(Evt::PermissionsStarted {
            count: device_perms.len(),
        });
        let mut diff_perms = HashMap::new();
        diff_perms.extend(diff_perms_lst.into_iter().map(|p| (p.name.clone(), p)));

        for d in device_perms.iter() {
            self.do_permission_diff(d, diff_perms.get(&d.name))?
        }
        self.trigger(Evt::PermissionsEnded);

        Ok(())
    }

    fn do_permission_diff(&self, perm: &Permission, diff: Option<&Permission>) -> DiffResult<()> {
        let diff_perm = diff.map(|it| it.protection_level.as_str());

        let matches = diff_perm
            .as_ref()
            .map_or(false, |it| perm.protection_level == *it);

        let ins = InsertPermissionDiff {
            permission: perm.id,
            diff_source: self.source.id,
            exists_in_diff: diff.is_some(),
            protection_level_matches_diff: matches,
            diff_protection_level: diff_perm,
        };
        self.db.add_permission_diff(&ins)?;
        self.trigger(Evt::Permission {
            id: perm.id,
            permission: perm.name.clone(),
            exists: diff.is_some(),
            matches,
        });
        Ok(())
    }

    fn diff_providers(&self) -> DiffResult<()> {
        let device = self.get_diffable_providers()?;
        let diff_lst = self.diff_db.get_providers()?;
        self.trigger(Evt::ProvidersStarted {
            count: device.len(),
        });
        let mut diff = HashMap::new();
        diff.extend(diff_lst.into_iter().map(|it| (it.name.clone(), it)));

        for d in device.iter() {
            self.do_provider_diff(d, diff.get(&d.name))?
        }
        self.trigger(Evt::ProvidersEnded);

        Ok(())
    }

    fn do_provider_diff(&self, device: &Provider, diff: Option<&Provider>) -> DiffResult<()> {
        let diff = match diff {
            Some(v) => v,
            None => return self.do_provider_not_in_diff(device),
        };

        let exported_matches = if device.exported {
            diff.exported
        } else {
            !diff.exported
        };

        let (perm_matches, diff_perm) =
            get_and_cmp(device.permission.as_ref(), diff.permission.as_ref());
        let (read_perm_matches, read_diff_perm) = get_and_cmp(
            device.read_permission.as_ref(),
            diff.read_permission.as_ref(),
        );
        let (write_perm_matches, write_diff_perm) = get_and_cmp(
            device.write_permission.as_ref(),
            diff.write_permission.as_ref(),
        );

        let ins = InsertProviderDiff {
            provider: device.id,
            diff_source: self.source.id,
            exists_in_diff: true,
            exported_matches_diff: exported_matches,
            permission_matches_diff: perm_matches,
            diff_permission: diff_perm,
            read_permission_matches_diff: read_perm_matches,
            diff_read_permission: read_diff_perm,
            write_permission_matches_diff: write_perm_matches,
            diff_write_permission: write_diff_perm,
        };

        self.db.add_provider_diff(&ins)?;

        self.trigger(Evt::Provider {
            id: device.id,
            authorities: device.authorities.clone(),
            exists: true,
        });

        Ok(())
    }

    fn do_provider_not_in_diff(&self, device: &Provider) -> DiffResult<()> {
        let ins = InsertProviderDiff {
            provider: device.id,
            diff_source: self.source.id,
            exists_in_diff: false,
            exported_matches_diff: false,
            permission_matches_diff: false,
            diff_permission: None,
            read_permission_matches_diff: false,
            diff_read_permission: None,
            write_permission_matches_diff: false,
            diff_write_permission: None,
        };
        self.db.add_provider_diff(&ins)?;
        self.trigger(Evt::Provider {
            id: device.id,
            authorities: device.authorities.clone(),
            exists: false,
        });

        Ok(())
    }

    fn trigger(&self, evt: DiffEvent) {
        self.monitor.on_event(evt);
    }
}

fn get_and_cmp<'a>(dev: Option<&'a String>, diff: Option<&'a String>) -> (bool, Option<&'a str>) {
    match diff {
        Some(diff) => match dev {
            Some(dev) => (diff == dev, Some(diff)),
            None => (false, Some(diff)),
        },
        None => (dev.is_none(), None),
    }
}
