use crate::{
    context::PyContext,
    exception::DtuError,
    types::{PyClassName, PyDevicePath, PyUnknownBool},
};
use dtu::db::sql::{
    device::{get_default_devicedb, models::*},
    DefaultDeviceDatabase, DeviceDatabase,
};
use pyo3::prelude::*;

#[pyclass]
pub struct DeviceDB(DefaultDeviceDatabase);

struct DBError(dtu::db::sql::Error);

impl From<dtu::db::sql::Error> for DBError {
    fn from(value: dtu::db::sql::Error) -> Self {
        Self(value)
    }
}

impl From<DBError> for PyErr {
    fn from(value: DBError) -> Self {
        DtuError::new_err(value.0.to_string())
    }
}

type Result<T> = std::result::Result<T, DBError>;

#[pymethods]
impl DeviceDB {
    #[new]
    fn new(pctx: &PyContext) -> Result<Self> {
        Ok(Self(get_default_devicedb(pctx)?))
    }

    #[pyo3(name = "get_permission_by_id")]
    fn py_get_permission_by_id(&self, sel: i32) -> Result<PyPermission> {
        self.get_permission_by_id(sel)
    }
    #[pyo3(name = "get_permissions")]
    fn py_get_permissions(&self) -> Result<Vec<PyPermission>> {
        self.get_permissions()
    }
    #[pyo3(name = "get_provider_by_id")]
    fn py_get_provider_by_id(&self, sel: i32) -> Result<PyProvider> {
        self.get_provider_by_id(sel)
    }
    #[pyo3(name = "get_providers")]
    fn py_get_providers(&self) -> Result<Vec<PyProvider>> {
        self.get_providers()
    }
    #[pyo3(name = "get_service_by_id")]
    fn py_get_service_by_id(&self, sel: i32) -> Result<PyService> {
        self.get_service_by_id(sel)
    }
    #[pyo3(name = "get_services")]
    fn py_get_services(&self) -> Result<Vec<PyService>> {
        self.get_services()
    }
    #[pyo3(name = "get_receiver_by_id")]
    fn py_get_receiver_by_id(&self, sel: i32) -> Result<PyReceiver> {
        self.get_receiver_by_id(sel)
    }
    #[pyo3(name = "get_receivers")]
    fn py_get_receivers(&self) -> Result<Vec<PyReceiver>> {
        self.get_receivers()
    }
    #[pyo3(name = "get_activity_by_id")]
    fn py_get_activity_by_id(&self, sel: i32) -> Result<PyActivity> {
        self.get_activity_by_id(sel)
    }
    #[pyo3(name = "get_activities")]
    fn py_get_activities(&self) -> Result<Vec<PyActivity>> {
        self.get_activities()
    }
    #[pyo3(name = "get_system_service_by_id")]
    fn py_get_system_service_by_id(&self, sel: i32) -> Result<PySystemService> {
        self.get_system_service_by_id(sel)
    }
    #[pyo3(name = "get_system_services")]
    fn py_get_system_services(&self) -> Result<Vec<PySystemService>> {
        self.get_system_services()
    }
    #[pyo3(name = "get_system_services_name_like")]
    fn py_get_system_services_name_like(&self, sel: &str) -> Result<Vec<PySystemService>> {
        self.get_system_services_name_like(sel)
    }
    #[pyo3(name = "get_system_service_by_name")]
    fn py_get_system_service_by_name(&self, sel: &str) -> Result<PySystemService> {
        self.get_system_service_by_name(sel)
    }
    #[pyo3(name = "get_system_service_methods")]
    fn py_get_system_service_methods(&self) -> Result<Vec<PySystemServiceMethod>> {
        self.get_system_service_methods()
    }
    #[pyo3(name = "get_device_property_by_id")]
    fn py_get_device_property_by_id(&self, sel: i32) -> Result<PyDeviceProperty> {
        self.get_device_property_by_id(sel)
    }
    #[pyo3(name = "get_device_properties")]
    fn py_get_device_properties(&self) -> Result<Vec<PyDeviceProperty>> {
        self.get_device_properties()
    }
    #[pyo3(name = "get_device_property_by_name")]
    fn py_get_device_property_by_name(&self, sel: &str) -> Result<PyDeviceProperty> {
        self.get_device_property_by_name(sel)
    }
    #[pyo3(name = "get_device_properties_like")]
    fn py_get_device_properties_like(&self, sel: &str) -> Result<Vec<PyDeviceProperty>> {
        self.get_device_properties_like(sel)
    }
    #[pyo3(name = "get_apk_by_id")]
    fn py_get_apk_by_id(&self, sel: i32) -> Result<PyApk> {
        self.get_apk_by_id(sel)
    }
    #[pyo3(name = "get_apks")]
    fn py_get_apks(&self) -> Result<Vec<PyApk>> {
        self.get_apks()
    }
    #[pyo3(name = "get_debuggable_apks")]
    fn py_get_debuggable_apks(&self) -> Result<Vec<PyApk>> {
        self.get_debuggable_apks()
    }
    #[pyo3(name = "get_apk_by_app_name")]
    fn py_get_apk_by_app_name(&self, sel: &str) -> Result<PyApk> {
        self.get_apk_by_app_name(sel)
    }
    #[pyo3(name = "get_apk_by_apk_name")]
    fn py_get_apk_by_apk_name(&self, sel: &str) -> Result<PyApk> {
        self.get_apk_by_apk_name(sel)
    }
    #[pyo3(name = "get_apk_by_device_path")]
    fn py_get_apk_by_device_path(&self, sel: &str) -> Result<PyApk> {
        self.get_apk_by_device_path(sel)
    }
    #[pyo3(name = "get_normal_permissions")]
    fn py_get_normal_permissions(&self) -> Result<Vec<PyPermission>> {
        self.get_normal_permissions()
    }
    #[pyo3(name = "get_permission_by_apk")]
    fn py_get_permission_by_apk(&self, sel: i32) -> Result<PyPermission> {
        self.get_permission_by_apk(sel)
    }
    #[pyo3(name = "get_permission_by_name")]
    fn py_get_permission_by_name(&self, sel: &str) -> Result<PyPermission> {
        self.get_permission_by_name(sel)
    }
    #[pyo3(name = "get_permissions_by_name_like")]
    fn py_get_permissions_by_name_like(&self, sel: &str) -> Result<Vec<PyPermission>> {
        self.get_permissions_by_name_like(sel)
    }
    #[pyo3(name = "get_system_service_impls")]
    fn py_get_system_service_impls(&self, sel: i32) -> Result<Vec<PySystemServiceImpl>> {
        self.get_system_service_impls(sel)
    }
    #[pyo3(name = "get_system_service_methods_by_service_id")]
    fn py_get_system_service_methods_by_service_id(
        &self,
        sel: i32,
    ) -> Result<Vec<PySystemServiceMethod>> {
        self.get_system_service_methods_by_service_id(sel)
    }
    #[pyo3(name = "get_provider_containing_authority")]
    fn py_get_provider_containing_authority(&self, sel: &str) -> Result<PyProvider> {
        self.get_provider_containing_authority(sel)
    }
    #[pyo3(name = "get_receivers_by_apk_id")]
    fn py_get_receivers_by_apk_id(&self, sel: i32) -> Result<Vec<PyReceiver>> {
        self.get_receivers_by_apk_id(sel)
    }
    #[pyo3(name = "get_services_by_apk_id")]
    fn py_get_services_by_apk_id(&self, sel: i32) -> Result<Vec<PyService>> {
        self.get_services_by_apk_id(sel)
    }
    #[pyo3(name = "get_activities_by_apk_id")]
    fn py_get_activities_by_apk_id(&self, sel: i32) -> Result<Vec<PyActivity>> {
        self.get_activities_by_apk_id(sel)
    }
    #[pyo3(name = "get_providers_by_apk_id")]
    fn py_get_providers_by_apk_id(&self, sel: i32) -> Result<Vec<PyProvider>> {
        self.get_providers_by_apk_id(sel)
    }
    #[pyo3(name = "get_diff_sources")]
    fn py_get_diff_sources(&self) -> Result<Vec<PyDiffSource>> {
        self.get_diff_sources()
    }
    #[pyo3(name = "get_diff_source_by_name")]
    fn py_get_diff_source_by_name(&self, sel: &str) -> Result<PyDiffSource> {
        self.get_diff_source_by_name(sel)
    }
    #[pyo3(name = "get_permission_diffs_by_diff_id")]
    fn py_get_permission_diffs_by_diff_id(&self, sel: i32) -> Result<Vec<PyDiffedPermission>> {
        self.get_permission_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_permission_diffs_by_diff_name")]
    fn py_get_permission_diffs_by_diff_name(&self, sel: &str) -> Result<Vec<PyDiffedPermission>> {
        self.get_permission_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_apk_diffs_by_diff_id")]
    fn py_get_apk_diffs_by_diff_id(&self, sel: i32) -> Result<Vec<PyDiffedApk>> {
        self.get_apk_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_apk_diffs_by_diff_name")]
    fn py_get_apk_diffs_by_diff_name(&self, sel: &str) -> Result<Vec<PyDiffedApk>> {
        self.get_apk_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_system_service_diffs_by_diff_id")]
    fn py_get_system_service_diffs_by_diff_id(
        &self,
        sel: i32,
    ) -> Result<Vec<PyDiffedSystemService>> {
        self.get_system_service_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_system_service_diffs_by_diff_name")]
    fn py_get_system_service_diffs_by_diff_name(
        &self,
        sel: &str,
    ) -> Result<Vec<PyDiffedSystemService>> {
        self.get_system_service_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_system_service_method_diffs_by_diff_id")]
    fn py_get_system_service_method_diffs_by_diff_id(
        &self,
        sel: i32,
    ) -> Result<Vec<PyDiffedSystemServiceMethod>> {
        self.get_system_service_method_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_system_service_method_diffs_by_diff_name")]
    fn py_get_system_service_method_diffs_by_diff_name(
        &self,
        sel: &str,
    ) -> Result<Vec<PyDiffedSystemServiceMethod>> {
        self.get_system_service_method_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_system_service_method_diffs_for_service")]
    fn py_get_system_service_method_diffs_for_service(
        &self,
        owner_id: i32,
        diff_id: i32,
    ) -> Result<Vec<PyDiffedSystemServiceMethod>> {
        self.get_system_service_method_diffs_for_service(owner_id, diff_id)
    }
    #[pyo3(name = "get_service_diffs_by_diff_id")]
    fn py_get_service_diffs_by_diff_id(&self, sel: i32) -> Result<Vec<PyDiffedService>> {
        self.get_service_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_service_diffs_by_diff_name")]
    fn py_get_service_diffs_by_diff_name(&self, sel: &str) -> Result<Vec<PyDiffedService>> {
        self.get_service_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_service_diffs_for_apk")]
    fn py_get_service_diffs_for_apk(
        &self,
        owner_id: i32,
        diff_id: i32,
    ) -> Result<Vec<PyDiffedService>> {
        self.get_service_diffs_for_apk(owner_id, diff_id)
    }
    #[pyo3(name = "get_provider_diffs_by_diff_id")]
    fn py_get_provider_diffs_by_diff_id(&self, sel: i32) -> Result<Vec<PyDiffedProvider>> {
        self.get_provider_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_provider_diffs_by_diff_name")]
    fn py_get_provider_diffs_by_diff_name(&self, sel: &str) -> Result<Vec<PyDiffedProvider>> {
        self.get_provider_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_provider_diffs_for_apk")]
    fn py_get_provider_diffs_for_apk(
        &self,
        owner_id: i32,
        diff_id: i32,
    ) -> Result<Vec<PyDiffedProvider>> {
        self.get_provider_diffs_for_apk(owner_id, diff_id)
    }
    #[pyo3(name = "get_activity_diffs_by_diff_id")]
    fn py_get_activity_diffs_by_diff_id(&self, sel: i32) -> Result<Vec<PyDiffedActivity>> {
        self.get_activity_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_activity_diffs_by_diff_name")]
    fn py_get_activity_diffs_by_diff_name(&self, sel: &str) -> Result<Vec<PyDiffedActivity>> {
        self.get_activity_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_activity_diffs_for_apk")]
    fn py_get_activity_diffs_for_apk(
        &self,
        owner_id: i32,
        diff_id: i32,
    ) -> Result<Vec<PyDiffedActivity>> {
        self.get_activity_diffs_for_apk(owner_id, diff_id)
    }
    #[pyo3(name = "get_receiver_diffs_by_diff_id")]
    fn py_get_receiver_diffs_by_diff_id(&self, sel: i32) -> Result<Vec<PyDiffedReceiver>> {
        self.get_receiver_diffs_by_diff_id(sel)
    }
    #[pyo3(name = "get_receiver_diffs_by_diff_name")]
    fn py_get_receiver_diffs_by_diff_name(&self, sel: &str) -> Result<Vec<PyDiffedReceiver>> {
        self.get_receiver_diffs_by_diff_name(sel)
    }
    #[pyo3(name = "get_receiver_diffs_for_apk")]
    fn py_get_receiver_diffs_for_apk(
        &self,
        owner_id: i32,
        diff_id: i32,
    ) -> Result<Vec<PyDiffedReceiver>> {
        self.get_receiver_diffs_for_apk(owner_id, diff_id)
    }
    #[pyo3(name = "get_fuzz_result_by_id")]
    fn py_get_fuzz_result_by_id(&self, sel: i32) -> Result<PyFuzzResult> {
        self.get_fuzz_result_by_id(sel)
    }
    #[pyo3(name = "get_fuzz_results")]
    fn py_get_fuzz_results(&self) -> Result<Vec<PyFuzzResult>> {
        self.get_fuzz_results()
    }
    #[pyo3(name = "get_endpoints_by_security")]
    fn py_get_endpoints_by_security(&self, sel: bool) -> Result<Vec<PyFuzzResult>> {
        self.get_endpoints_by_security(sel)
    }
}

macro_rules! py_call_get_multi {
    ($name:ident, $ret:ident) => {
        fn $name(&self) -> Result<Vec<$ret>> {
            Ok(self.0.$name()?.into_iter().map($ret::from).collect())
        }
    };
}

macro_rules! py_call_get_one_by {
    ($name:ident, $sel:ty, $ret:ident) => {
        fn $name(&self, sel: $sel) -> Result<$ret> {
            Ok($ret::from(self.0.$name(sel)?))
        }
    };
}

macro_rules! py_call_get_multi_by {
    ($name:ident, $sel:ty, $ret:ident) => {
        fn $name(&self, sel: $sel) -> Result<Vec<$ret>> {
            Ok(self.0.$name(sel)?.into_iter().map($ret::from).collect())
        }
    };
}

macro_rules! py_call_simple_get {
    (
        $get_all:ident,
        $get_by_id:ident,
        $read_type:ident
    ) => {
        py_call_get_one_by!($get_by_id, i32, $read_type);
        py_call_get_multi!($get_all, $read_type);
    };
}

macro_rules! py_call_diff_item {
    (
        $get_all_by_diff_name:ident,
        $get_all_by_diff_id:ident,
        $get_type:ident
    ) => {
        py_call_get_multi_by!($get_all_by_diff_id, i32, $get_type);
        py_call_get_multi_by!($get_all_by_diff_name, &str, $get_type);
    };

    (
        $get_all_by_diff_name:ident,
        $get_all_by_diff_id:ident,
        $get_type:ident,
        $get_by_two_ids:ident
    ) => {
        py_call_diff_item!($get_all_by_diff_name, $get_all_by_diff_id, $get_type);
        fn $get_by_two_ids(&self, owner_id: i32, diff_id: i32) -> Result<Vec<$get_type>> {
            Ok(self
                .0
                .$get_by_two_ids(owner_id, diff_id)?
                .into_iter()
                .map($get_type::from)
                .collect())
        }
    };
}

impl DeviceDB {
    py_call_simple_get!(get_permissions, get_permission_by_id, PyPermission);

    py_call_simple_get!(get_providers, get_provider_by_id, PyProvider);

    py_call_simple_get!(get_services, get_service_by_id, PyService);

    py_call_simple_get!(get_receivers, get_receiver_by_id, PyReceiver);

    py_call_simple_get!(get_activities, get_activity_by_id, PyActivity);

    py_call_simple_get!(
        get_system_services,
        get_system_service_by_id,
        PySystemService
    );

    py_call_get_multi_by!(get_system_services_name_like, &str, PySystemService);
    py_call_get_one_by!(get_system_service_by_name, &str, PySystemService);
    py_call_get_multi!(get_system_service_methods, PySystemServiceMethod);

    py_call_simple_get!(
        get_device_properties,
        get_device_property_by_id,
        PyDeviceProperty
    );

    py_call_get_one_by!(get_device_property_by_name, &str, PyDeviceProperty);
    py_call_get_multi_by!(get_device_properties_like, &str, PyDeviceProperty);

    py_call_simple_get!(get_apks, get_apk_by_id, PyApk);

    py_call_get_multi!(get_debuggable_apks, PyApk);
    py_call_get_one_by!(get_apk_by_app_name, &str, PyApk);
    py_call_get_one_by!(get_apk_by_apk_name, &str, PyApk);
    py_call_get_one_by!(get_apk_by_device_path, &str, PyApk);
    py_call_get_multi!(get_normal_permissions, PyPermission);
    py_call_get_one_by!(get_permission_by_apk, i32, PyPermission);
    py_call_get_one_by!(get_permission_by_name, &str, PyPermission);
    py_call_get_multi_by!(get_permissions_by_name_like, &str, PyPermission);
    py_call_get_multi_by!(get_system_service_impls, i32, PySystemServiceImpl);

    py_call_get_multi_by!(
        get_system_service_methods_by_service_id,
        i32,
        PySystemServiceMethod
    );

    py_call_get_one_by!(get_provider_containing_authority, &str, PyProvider);
    py_call_get_multi_by!(get_receivers_by_apk_id, i32, PyReceiver);
    py_call_get_multi_by!(get_services_by_apk_id, i32, PyService);
    py_call_get_multi_by!(get_activities_by_apk_id, i32, PyActivity);
    py_call_get_multi_by!(get_providers_by_apk_id, i32, PyProvider);

    py_call_get_multi!(get_diff_sources, PyDiffSource);
    py_call_get_one_by!(get_diff_source_by_name, &str, PyDiffSource);

    py_call_diff_item!(
        get_permission_diffs_by_diff_name,
        get_permission_diffs_by_diff_id,
        PyDiffedPermission
    );

    py_call_diff_item!(
        get_apk_diffs_by_diff_name,
        get_apk_diffs_by_diff_id,
        PyDiffedApk
    );

    py_call_diff_item!(
        get_system_service_diffs_by_diff_name,
        get_system_service_diffs_by_diff_id,
        PyDiffedSystemService
    );

    py_call_diff_item!(
        get_system_service_method_diffs_by_diff_name,
        get_system_service_method_diffs_by_diff_id,
        PyDiffedSystemServiceMethod,
        get_system_service_method_diffs_for_service
    );

    py_call_diff_item!(
        get_service_diffs_by_diff_name,
        get_service_diffs_by_diff_id,
        PyDiffedService,
        get_service_diffs_for_apk
    );

    py_call_diff_item!(
        get_provider_diffs_by_diff_name,
        get_provider_diffs_by_diff_id,
        PyDiffedProvider,
        get_provider_diffs_for_apk
    );

    py_call_diff_item!(
        get_activity_diffs_by_diff_name,
        get_activity_diffs_by_diff_id,
        PyDiffedActivity,
        get_activity_diffs_for_apk
    );

    py_call_diff_item!(
        get_receiver_diffs_by_diff_name,
        get_receiver_diffs_by_diff_id,
        PyDiffedReceiver,
        get_receiver_diffs_for_apk
    );

    py_call_simple_get!(get_fuzz_results, get_fuzz_result_by_id, PyFuzzResult);
    py_call_get_multi_by!(get_endpoints_by_security, bool, PyFuzzResult);
}

#[pyclass(frozen, name = "DeviceProperty")]
#[derive(Clone)]
pub struct PyDeviceProperty(pub(crate) DeviceProperty);

impl From<DeviceProperty> for PyDeviceProperty {
    fn from(value: DeviceProperty) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDeviceProperty {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn value(&self) -> &str {
        &self.0.value
    }
}

#[pyclass(frozen, name = "Permission")]
#[derive(Clone)]
pub struct PyPermission(pub(crate) Permission);

impl From<Permission> for PyPermission {
    fn from(value: Permission) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyPermission {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn protection_level(&self) -> &str {
        &self.0.protection_level
    }
    #[getter]
    fn source_apk_id(&self) -> i32 {
        self.0.source_apk_id
    }
}

#[pyclass(frozen, name = "ApkPermission")]
#[derive(Clone)]
pub struct PyApkPermission(pub(crate) ApkPermission);

impl From<ApkPermission> for PyApkPermission {
    fn from(value: ApkPermission) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApkPermission {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn apk_id(&self) -> i32 {
        self.0.apk_id
    }
}

#[pyclass(frozen, name = "PermissionDiff")]
#[derive(Clone)]
pub struct PyPermissionDiff(pub(crate) PermissionDiff);

impl From<PermissionDiff> for PyPermissionDiff {
    fn from(value: PermissionDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyPermissionDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn permission(&self) -> i32 {
        self.0.permission
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn protection_level_matches_diff(&self) -> bool {
        self.0.protection_level_matches_diff
    }
    #[getter]
    fn diff_protection_level(&self) -> Option<&str> {
        self.0.diff_protection_level.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "DiffedPermission")]
#[derive(Clone)]
pub struct PyDiffedPermission(pub(crate) DiffedPermission);

impl From<DiffedPermission> for PyDiffedPermission {
    fn from(value: DiffedPermission) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedPermission {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn permission(&self) -> PyPermission {
        PyPermission(self.0.permission.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn protection_level_matches_diff(&self) -> bool {
        self.0.protection_level_matches_diff
    }
    #[getter]
    fn diff_protection_level(&self) -> Option<&str> {
        self.0.diff_protection_level.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "ProtectedBroadcast")]
#[derive(Clone)]
pub struct PyProtectedBroadcast(pub(crate) ProtectedBroadcast);

impl From<ProtectedBroadcast> for PyProtectedBroadcast {
    fn from(value: ProtectedBroadcast) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyProtectedBroadcast {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
}

#[pyclass(frozen, name = "UnprotectedBroadcast")]
#[derive(Clone)]
pub struct PyUnprotectedBroadcast(pub(crate) UnprotectedBroadcast);

impl From<UnprotectedBroadcast> for PyUnprotectedBroadcast {
    fn from(value: UnprotectedBroadcast) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyUnprotectedBroadcast {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
}

#[pyclass(frozen, name = "Apk")]
#[derive(Clone)]
pub struct PyApk(pub(crate) Apk);

impl From<Apk> for PyApk {
    fn from(value: Apk) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApk {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn app_name(&self) -> &str {
        &self.0.app_name
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn is_debuggable(&self) -> bool {
        self.0.is_debuggable
    }
    #[getter]
    fn is_priv(&self) -> bool {
        self.0.is_priv
    }
    #[getter]
    fn device_path(&self) -> PyDevicePath {
        self.0.device_path.clone().into()
    }
}

#[pyclass(frozen, name = "ApkWithPermissions")]
#[derive(Clone)]
pub struct PyApkWithPermissions(pub(crate) ApkWithPermissions);

impl From<ApkWithPermissions> for PyApkWithPermissions {
    fn from(value: ApkWithPermissions) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApkWithPermissions {
    #[getter]
    fn apk(&self) -> PyApk {
        PyApk(self.0.apk.clone())
    }
    #[getter]
    fn permissions(&self) -> Vec<String> {
        self.0.permissions.clone()
    }
}

#[pyclass(frozen, name = "DiffedApk")]
#[derive(Clone)]
pub struct PyDiffedApk(pub(crate) DiffedApk);

impl From<DiffedApk> for PyDiffedApk {
    fn from(value: DiffedApk) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedApk {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn apk(&self) -> PyApk {
        PyApk(self.0.apk.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
}

#[pyclass(frozen, name = "ApkDiff")]
#[derive(Clone)]
pub struct PyApkDiff(pub(crate) ApkDiff);

impl From<ApkDiff> for PyApkDiff {
    fn from(value: ApkDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApkDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn apk(&self) -> i32 {
        self.0.apk
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
}

#[pyclass(frozen, name = "Receiver")]
#[derive(Clone)]
pub struct PyReceiver(pub(crate) Receiver);

impl From<Receiver> for PyReceiver {
    fn from(value: Receiver) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyReceiver {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn class_name(&self) -> PyClassName {
        self.0.class_name.clone().into()
    }
    #[getter]
    fn exported(&self) -> bool {
        self.0.exported
    }
    #[getter]
    fn enabled(&self) -> bool {
        self.0.enabled
    }
    #[getter]
    fn pkg(&self) -> &str {
        &self.0.pkg
    }
    #[getter]
    fn apk_id(&self) -> i32 {
        self.0.apk_id
    }
    #[getter]
    fn permission(&self) -> Option<&str> {
        self.0.permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "ReceiverDiff")]
#[derive(Clone)]
pub struct PyReceiverDiff(pub(crate) ReceiverDiff);

impl From<ReceiverDiff> for PyReceiverDiff {
    fn from(value: ReceiverDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyReceiverDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn receiver(&self) -> i32 {
        self.0.receiver
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "DiffedReceiver")]
#[derive(Clone)]
pub struct PyDiffedReceiver(pub(crate) DiffedReceiver);

impl From<DiffedReceiver> for PyDiffedReceiver {
    fn from(value: DiffedReceiver) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedReceiver {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn receiver(&self) -> PyReceiver {
        PyReceiver(self.0.receiver.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "Service")]
#[derive(Clone)]
pub struct PyService(pub(crate) Service);

impl From<Service> for PyService {
    fn from(value: Service) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyService {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn class_name(&self) -> PyClassName {
        self.0.class_name.clone().into()
    }
    #[getter]
    fn exported(&self) -> bool {
        self.0.exported
    }
    #[getter]
    fn enabled(&self) -> bool {
        self.0.enabled
    }
    #[getter]
    fn pkg(&self) -> &str {
        &self.0.pkg
    }
    #[getter]
    fn apk_id(&self) -> i32 {
        self.0.apk_id
    }
    #[getter]
    fn returns_binder(&self) -> PyUnknownBool {
        self.0.returns_binder.into()
    }
    #[getter]
    fn permission(&self) -> Option<&str> {
        self.0.permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "ServiceDiff")]
#[derive(Clone)]
pub struct PyServiceDiff(pub(crate) ServiceDiff);

impl From<ServiceDiff> for PyServiceDiff {
    fn from(value: ServiceDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyServiceDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn service(&self) -> i32 {
        self.0.service
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "DiffedService")]
#[derive(Clone)]
pub struct PyDiffedService(pub(crate) DiffedService);

impl From<DiffedService> for PyDiffedService {
    fn from(value: DiffedService) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedService {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn service(&self) -> PyService {
        PyService(self.0.service.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "Activity")]
#[derive(Clone)]
pub struct PyActivity(pub(crate) Activity);

impl From<Activity> for PyActivity {
    fn from(value: Activity) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyActivity {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn class_name(&self) -> PyClassName {
        self.0.class_name.clone().into()
    }
    #[getter]
    fn exported(&self) -> bool {
        self.0.exported
    }
    #[getter]
    fn enabled(&self) -> bool {
        self.0.enabled
    }
    #[getter]
    fn pkg(&self) -> &str {
        &self.0.pkg
    }
    #[getter]
    fn apk_id(&self) -> i32 {
        self.0.apk_id
    }
    #[getter]
    fn permission(&self) -> Option<&str> {
        self.0.permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "ActivityDiff")]
#[derive(Clone)]
pub struct PyActivityDiff(pub(crate) ActivityDiff);

impl From<ActivityDiff> for PyActivityDiff {
    fn from(value: ActivityDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyActivityDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn activity(&self) -> i32 {
        self.0.activity
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "DiffedActivity")]
#[derive(Clone)]
pub struct PyDiffedActivity(pub(crate) DiffedActivity);

impl From<DiffedActivity> for PyDiffedActivity {
    fn from(value: DiffedActivity) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedActivity {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn activity(&self) -> PyActivity {
        PyActivity(self.0.activity.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "Provider")]
#[derive(Clone)]
pub struct PyProvider(pub(crate) Provider);

impl From<Provider> for PyProvider {
    fn from(value: Provider) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyProvider {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn authorities(&self) -> &str {
        &self.0.authorities
    }
    #[getter]
    fn grant_uri_permissions(&self) -> bool {
        self.0.grant_uri_permissions
    }
    #[getter]
    fn exported(&self) -> bool {
        self.0.exported
    }
    #[getter]
    fn enabled(&self) -> bool {
        self.0.enabled
    }
    #[getter]
    fn apk_id(&self) -> i32 {
        self.0.apk_id
    }
    #[getter]
    fn permission(&self) -> Option<&str> {
        self.0.permission.as_ref().map(String::as_str)
    }
    #[getter]
    fn read_permission(&self) -> Option<&str> {
        self.0.read_permission.as_ref().map(String::as_str)
    }
    #[getter]
    fn write_permission(&self) -> Option<&str> {
        self.0.write_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "ProviderDiff")]
#[derive(Clone)]
pub struct PyProviderDiff(pub(crate) ProviderDiff);

impl From<ProviderDiff> for PyProviderDiff {
    fn from(value: ProviderDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyProviderDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn provider(&self) -> i32 {
        self.0.provider
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn write_permission_matches_diff(&self) -> bool {
        self.0.write_permission_matches_diff
    }
    #[getter]
    fn read_permission_matches_diff(&self) -> bool {
        self.0.read_permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
    #[getter]
    fn diff_write_permission(&self) -> Option<&str> {
        self.0.diff_write_permission.as_ref().map(String::as_str)
    }
    #[getter]
    fn diff_read_permission(&self) -> Option<&str> {
        self.0.diff_read_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "DiffedProvider")]
#[derive(Clone)]
pub struct PyDiffedProvider(pub(crate) DiffedProvider);

impl From<DiffedProvider> for PyDiffedProvider {
    fn from(value: DiffedProvider) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedProvider {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn provider(&self) -> PyProvider {
        PyProvider(self.0.provider.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn exported_matches_diff(&self) -> bool {
        self.0.exported_matches_diff
    }
    #[getter]
    fn permission_matches_diff(&self) -> bool {
        self.0.permission_matches_diff
    }
    #[getter]
    fn write_permission_matches_diff(&self) -> bool {
        self.0.write_permission_matches_diff
    }
    #[getter]
    fn read_permission_matches_diff(&self) -> bool {
        self.0.read_permission_matches_diff
    }
    #[getter]
    fn diff_permission(&self) -> Option<&str> {
        self.0.diff_permission.as_ref().map(String::as_str)
    }
    #[getter]
    fn diff_write_permission(&self) -> Option<&str> {
        self.0.diff_write_permission.as_ref().map(String::as_str)
    }
    #[getter]
    fn diff_read_permission(&self) -> Option<&str> {
        self.0.diff_read_permission.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "SystemServiceImpl")]
#[derive(Clone)]
pub struct PySystemServiceImpl(pub(crate) SystemServiceImpl);

impl From<SystemServiceImpl> for PySystemServiceImpl {
    fn from(value: SystemServiceImpl) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceImpl {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn system_service_id(&self) -> i32 {
        self.0.system_service_id
    }
    #[getter]
    fn source(&self) -> &str {
        &self.0.source
    }
    #[getter]
    fn class_name(&self) -> PyClassName {
        self.0.class_name.clone().into()
    }
}

#[pyclass(frozen, name = "SystemServiceMethod")]
#[derive(Clone)]
pub struct PySystemServiceMethod(pub(crate) SystemServiceMethod);

impl From<SystemServiceMethod> for PySystemServiceMethod {
    fn from(value: SystemServiceMethod) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceMethod {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn system_service_id(&self) -> i32 {
        self.0.system_service_id
    }
    #[getter]
    fn transaction_id(&self) -> i32 {
        self.0.transaction_id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn signature(&self) -> Option<&str> {
        self.0.signature.as_ref().map(String::as_str)
    }
    #[getter]
    fn return_type(&self) -> Option<&str> {
        self.0.return_type.as_ref().map(String::as_str)
    }
    #[getter]
    fn smalisa_hash(&self) -> Option<&str> {
        self.0.smalisa_hash.as_ref().map(String::as_str)
    }
}

#[pyclass(frozen, name = "SystemServiceMethodDiff")]
#[derive(Clone)]
pub struct PySystemServiceMethodDiff(pub(crate) SystemServiceMethodDiff);

impl From<SystemServiceMethodDiff> for PySystemServiceMethodDiff {
    fn from(value: SystemServiceMethodDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceMethodDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn method(&self) -> i32 {
        self.0.method
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn hash_matches_diff(&self) -> PyUnknownBool {
        PyUnknownBool(self.0.hash_matches_diff)
    }
}

#[pyclass(frozen, name = "DiffedSystemServiceMethod")]
#[derive(Clone)]
pub struct PyDiffedSystemServiceMethod(pub(crate) DiffedSystemServiceMethod);

impl From<DiffedSystemServiceMethod> for PyDiffedSystemServiceMethod {
    fn from(value: DiffedSystemServiceMethod) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedSystemServiceMethod {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn method(&self) -> PySystemServiceMethod {
        PySystemServiceMethod(self.0.method.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
    #[getter]
    fn hash_matches_diff(&self) -> PyUnknownBool {
        PyUnknownBool(self.0.hash_matches_diff)
    }
}

#[pyclass(frozen, name = "SystemService")]
#[derive(Clone)]
pub struct PySystemService(pub(crate) SystemService);

impl From<SystemService> for PySystemService {
    fn from(value: SystemService) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemService {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
    #[getter]
    fn can_get_binder(&self) -> PyUnknownBool {
        PyUnknownBool(self.0.can_get_binder)
    }
    #[getter]
    fn iface(&self) -> Option<PyClassName> {
        self.0.iface.clone().map(PyClassName::from)
    }
}

#[pyclass(frozen, name = "SystemServiceDiff")]
#[derive(Clone)]
pub struct PySystemServiceDiff(pub(crate) SystemServiceDiff);

impl From<SystemServiceDiff> for PySystemServiceDiff {
    fn from(value: SystemServiceDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceDiff {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn system_service(&self) -> i32 {
        self.0.system_service
    }
    #[getter]
    fn diff_source(&self) -> i32 {
        self.0.diff_source
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
}

#[pyclass(frozen, name = "DiffedSystemService")]
#[derive(Clone)]
pub struct PyDiffedSystemService(pub(crate) DiffedSystemService);

impl From<DiffedSystemService> for PyDiffedSystemService {
    fn from(value: DiffedSystemService) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedSystemService {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn service(&self) -> PySystemService {
        PySystemService(self.0.service.clone())
    }
    #[getter]
    fn exists_in_diff(&self) -> bool {
        self.0.exists_in_diff
    }
}

#[pyclass(frozen, name = "DiffSource")]
#[derive(Clone)]
pub struct PyDiffSource(pub(crate) DiffSource);

impl From<DiffSource> for PyDiffSource {
    fn from(value: DiffSource) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffSource {
    fn __str__(&self) -> String {
        format!("{}", self.0)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }
}

#[pyclass(frozen, name = "FuzzResult")]
#[derive(Clone)]
pub struct PyFuzzResult(pub(crate) FuzzResult);

impl From<FuzzResult> for PyFuzzResult {
    fn from(value: FuzzResult) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyFuzzResult {
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }
    #[getter]
    fn service_name(&self) -> &str {
        &self.0.service_name
    }
    #[getter]
    fn method_name(&self) -> &str {
        &self.0.method_name
    }
    #[getter]
    fn exception_thrown(&self) -> bool {
        self.0.exception_thrown
    }
    #[getter]
    fn security_exception_thrown(&self) -> bool {
        self.0.security_exception_thrown
    }
}
