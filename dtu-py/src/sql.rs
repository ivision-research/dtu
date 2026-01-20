use std::collections::HashMap;

use crate::{
    context::PyContext,
    exception::DtuError,
    types::{PyClassName, PyDevicePath, PyUnknownBool}, utils::{reduce, unpickle},
};
use dtu::db::sql::{
    device::{get_default_devicedb, models::*},
    DefaultDeviceDatabase, DeviceDatabase,
};
use pyo3::{prelude::*, types::PyTuple};

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

#[pyclass(module = "dtu")]
pub struct DeviceDB(DefaultDeviceDatabase);

/// Provide read only access to the Device database
#[pymethods]
impl DeviceDB {
    #[new]
    #[pyo3(signature = (ctx = None))]
    fn new(ctx: Option<&PyContext>) -> Result<Self> {
        Ok(Self(match ctx {
            Some(v) => get_default_devicedb(v),
            None => get_default_devicedb(&dtu::DefaultContext::new()),
        }?))
    }
    fn get_all_system_service_impls(&self) -> Result<HashMap<String, Vec<PySystemServiceImpl>>> {
        Ok(HashMap::from_iter(
            self.0
                .get_all_system_service_impls()?
                .into_iter()
                .map(|(service, impls)| {
                    (
                        service,
                        impls
                            .into_iter()
                            .map(PySystemServiceImpl::from)
                            .collect::<Vec<PySystemServiceImpl>>(),
                    )
                }),
        ))
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

#[pyclass(module = "dtu", frozen, name = "DeviceProperty")]
#[derive(Clone)]
pub struct PyDeviceProperty(pub(crate) DeviceProperty);

impl AsRef<DeviceProperty> for PyDeviceProperty {
    fn as_ref(&self) -> &DeviceProperty {
        &self.0
    }
}

impl From<DeviceProperty> for PyDeviceProperty {
    fn from(value: DeviceProperty) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDeviceProperty {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DeviceProperty, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DeviceProperty>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "Permission")]
#[derive(Clone)]
pub struct PyPermission(pub(crate) Permission);

impl AsRef<Permission> for PyPermission {
    fn as_ref(&self) -> &Permission {
        &self.0
    }
}

impl From<Permission> for PyPermission {
    fn from(value: Permission) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyPermission {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<Permission, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, Permission>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ApkPermission")]
#[derive(Clone)]
pub struct PyApkPermission(pub(crate) ApkPermission);

impl AsRef<ApkPermission> for PyApkPermission {
    fn as_ref(&self) -> &ApkPermission {
        &self.0
    }
}

impl From<ApkPermission> for PyApkPermission {
    fn from(value: ApkPermission) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApkPermission {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ApkPermission, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ApkPermission>(self, py)
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
    fn apk_id(&self) -> i32 {
        self.0.apk_id
    }
}

#[pyclass(module = "dtu", frozen, name = "PermissionDiff")]
#[derive(Clone)]
pub struct PyPermissionDiff(pub(crate) PermissionDiff);

impl AsRef<PermissionDiff> for PyPermissionDiff {
    fn as_ref(&self) -> &PermissionDiff {
        &self.0
    }
}

impl From<PermissionDiff> for PyPermissionDiff {
    fn from(value: PermissionDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyPermissionDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<PermissionDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, PermissionDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedPermission")]
#[derive(Clone)]
pub struct PyDiffedPermission(pub(crate) DiffedPermission);

impl AsRef<DiffedPermission> for PyDiffedPermission {
    fn as_ref(&self) -> &DiffedPermission {
        &self.0
    }
}

impl From<DiffedPermission> for PyDiffedPermission {
    fn from(value: DiffedPermission) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedPermission {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedPermission, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedPermission>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ProtectedBroadcast")]
#[derive(Clone)]
pub struct PyProtectedBroadcast(pub(crate) ProtectedBroadcast);

impl AsRef<ProtectedBroadcast> for PyProtectedBroadcast {
    fn as_ref(&self) -> &ProtectedBroadcast {
        &self.0
    }
}

impl From<ProtectedBroadcast> for PyProtectedBroadcast {
    fn from(value: ProtectedBroadcast) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyProtectedBroadcast {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ProtectedBroadcast, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ProtectedBroadcast>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "UnprotectedBroadcast")]
#[derive(Clone)]
pub struct PyUnprotectedBroadcast(pub(crate) UnprotectedBroadcast);

impl AsRef<UnprotectedBroadcast> for PyUnprotectedBroadcast {
    fn as_ref(&self) -> &UnprotectedBroadcast {
        &self.0
    }
}

impl From<UnprotectedBroadcast> for PyUnprotectedBroadcast {
    fn from(value: UnprotectedBroadcast) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyUnprotectedBroadcast {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<UnprotectedBroadcast, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, UnprotectedBroadcast>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "Apk")]
#[derive(Clone)]
pub struct PyApk(pub(crate) Apk);

impl AsRef<Apk> for PyApk {
    fn as_ref(&self) -> &Apk {
        &self.0
    }
}

impl From<Apk> for PyApk {
    fn from(value: Apk) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApk {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<Apk, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, Apk>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ApkWithPermissions")]
#[derive(Clone)]
pub struct PyApkWithPermissions(pub(crate) ApkWithPermissions);

impl AsRef<ApkWithPermissions> for PyApkWithPermissions {
    fn as_ref(&self) -> &ApkWithPermissions {
        &self.0
    }
}

impl From<ApkWithPermissions> for PyApkWithPermissions {
    fn from(value: ApkWithPermissions) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApkWithPermissions {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ApkWithPermissions, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ApkWithPermissions>(self, py)
    }
    #[getter]
    fn apk(&self) -> PyApk {
        PyApk(self.0.apk.clone())
    }
    #[getter]
    fn permissions(&self) -> Vec<String> {
        self.0.permissions.clone()
    }
}

#[pyclass(module = "dtu", frozen, name = "DiffedApk")]
#[derive(Clone)]
pub struct PyDiffedApk(pub(crate) DiffedApk);

impl AsRef<DiffedApk> for PyDiffedApk {
    fn as_ref(&self) -> &DiffedApk {
        &self.0
    }
}

impl From<DiffedApk> for PyDiffedApk {
    fn from(value: DiffedApk) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedApk {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedApk, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedApk>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ApkDiff")]
#[derive(Clone)]
pub struct PyApkDiff(pub(crate) ApkDiff);

impl AsRef<ApkDiff> for PyApkDiff {
    fn as_ref(&self) -> &ApkDiff {
        &self.0
    }
}

impl From<ApkDiff> for PyApkDiff {
    fn from(value: ApkDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyApkDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ApkDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ApkDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "Receiver")]
#[derive(Clone)]
pub struct PyReceiver(pub(crate) Receiver);

impl AsRef<Receiver> for PyReceiver {
    fn as_ref(&self) -> &Receiver {
        &self.0
    }
}

impl From<Receiver> for PyReceiver {
    fn from(value: Receiver) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyReceiver {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<Receiver, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, Receiver>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ReceiverDiff")]
#[derive(Clone)]
pub struct PyReceiverDiff(pub(crate) ReceiverDiff);

impl AsRef<ReceiverDiff> for PyReceiverDiff {
    fn as_ref(&self) -> &ReceiverDiff {
        &self.0
    }
}

impl From<ReceiverDiff> for PyReceiverDiff {
    fn from(value: ReceiverDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyReceiverDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ReceiverDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ReceiverDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedReceiver")]
#[derive(Clone)]
pub struct PyDiffedReceiver(pub(crate) DiffedReceiver);

impl AsRef<DiffedReceiver> for PyDiffedReceiver {
    fn as_ref(&self) -> &DiffedReceiver {
        &self.0
    }
}

impl From<DiffedReceiver> for PyDiffedReceiver {
    fn from(value: DiffedReceiver) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedReceiver {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedReceiver, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedReceiver>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "Service")]
#[derive(Clone)]
pub struct PyService(pub(crate) Service);

impl AsRef<Service> for PyService {
    fn as_ref(&self) -> &Service {
        &self.0
    }
}

impl From<Service> for PyService {
    fn from(value: Service) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyService {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<Service, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, Service>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ServiceDiff")]
#[derive(Clone)]
pub struct PyServiceDiff(pub(crate) ServiceDiff);

impl AsRef<ServiceDiff> for PyServiceDiff {
    fn as_ref(&self) -> &ServiceDiff {
        &self.0
    }
}

impl From<ServiceDiff> for PyServiceDiff {
    fn from(value: ServiceDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyServiceDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ServiceDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ServiceDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedService")]
#[derive(Clone)]
pub struct PyDiffedService(pub(crate) DiffedService);

impl AsRef<DiffedService> for PyDiffedService {
    fn as_ref(&self) -> &DiffedService {
        &self.0
    }
}

impl From<DiffedService> for PyDiffedService {
    fn from(value: DiffedService) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedService {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedService, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedService>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "Activity")]
#[derive(Clone)]
pub struct PyActivity(pub(crate) Activity);

impl AsRef<Activity> for PyActivity {
    fn as_ref(&self) -> &Activity {
        &self.0
    }
}

impl From<Activity> for PyActivity {
    fn from(value: Activity) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyActivity {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<Activity, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, Activity>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ActivityDiff")]
#[derive(Clone)]
pub struct PyActivityDiff(pub(crate) ActivityDiff);

impl AsRef<ActivityDiff> for PyActivityDiff {
    fn as_ref(&self) -> &ActivityDiff {
        &self.0
    }
}

impl From<ActivityDiff> for PyActivityDiff {
    fn from(value: ActivityDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyActivityDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ActivityDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ActivityDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedActivity")]
#[derive(Clone)]
pub struct PyDiffedActivity(pub(crate) DiffedActivity);

impl AsRef<DiffedActivity> for PyDiffedActivity {
    fn as_ref(&self) -> &DiffedActivity {
        &self.0
    }
}

impl From<DiffedActivity> for PyDiffedActivity {
    fn from(value: DiffedActivity) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedActivity {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedActivity, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedActivity>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "Provider")]
#[derive(Clone)]
pub struct PyProvider(pub(crate) Provider);

impl AsRef<Provider> for PyProvider {
    fn as_ref(&self) -> &Provider {
        &self.0
    }
}

impl From<Provider> for PyProvider {
    fn from(value: Provider) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyProvider {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<Provider, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, Provider>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "ProviderDiff")]
#[derive(Clone)]
pub struct PyProviderDiff(pub(crate) ProviderDiff);

impl AsRef<ProviderDiff> for PyProviderDiff {
    fn as_ref(&self) -> &ProviderDiff {
        &self.0
    }
}

impl From<ProviderDiff> for PyProviderDiff {
    fn from(value: ProviderDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyProviderDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ProviderDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ProviderDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedProvider")]
#[derive(Clone)]
pub struct PyDiffedProvider(pub(crate) DiffedProvider);

impl AsRef<DiffedProvider> for PyDiffedProvider {
    fn as_ref(&self) -> &DiffedProvider {
        &self.0
    }
}

impl From<DiffedProvider> for PyDiffedProvider {
    fn from(value: DiffedProvider) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedProvider {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedProvider, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedProvider>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "SystemServiceImpl")]
#[derive(Clone)]
pub struct PySystemServiceImpl(pub(crate) SystemServiceImpl);

impl AsRef<SystemServiceImpl> for PySystemServiceImpl {
    fn as_ref(&self) -> &SystemServiceImpl {
        &self.0
    }
}

impl From<SystemServiceImpl> for PySystemServiceImpl {
    fn from(value: SystemServiceImpl) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceImpl {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<SystemServiceImpl, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, SystemServiceImpl>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "SystemServiceMethod")]
#[derive(Clone)]
pub struct PySystemServiceMethod(pub(crate) SystemServiceMethod);

impl AsRef<SystemServiceMethod> for PySystemServiceMethod {
    fn as_ref(&self) -> &SystemServiceMethod {
        &self.0
    }
}

impl From<SystemServiceMethod> for PySystemServiceMethod {
    fn from(value: SystemServiceMethod) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceMethod {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<SystemServiceMethod, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, SystemServiceMethod>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "SystemServiceMethodDiff")]
#[derive(Clone)]
pub struct PySystemServiceMethodDiff(pub(crate) SystemServiceMethodDiff);

impl AsRef<SystemServiceMethodDiff> for PySystemServiceMethodDiff {
    fn as_ref(&self) -> &SystemServiceMethodDiff {
        &self.0
    }
}

impl From<SystemServiceMethodDiff> for PySystemServiceMethodDiff {
    fn from(value: SystemServiceMethodDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceMethodDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<SystemServiceMethodDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, SystemServiceMethodDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedSystemServiceMethod")]
#[derive(Clone)]
pub struct PyDiffedSystemServiceMethod(pub(crate) DiffedSystemServiceMethod);

impl AsRef<DiffedSystemServiceMethod> for PyDiffedSystemServiceMethod {
    fn as_ref(&self) -> &DiffedSystemServiceMethod {
        &self.0
    }
}

impl From<DiffedSystemServiceMethod> for PyDiffedSystemServiceMethod {
    fn from(value: DiffedSystemServiceMethod) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedSystemServiceMethod {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedSystemServiceMethod, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedSystemServiceMethod>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "SystemService")]
#[derive(Clone)]
pub struct PySystemService(pub(crate) SystemService);

impl AsRef<SystemService> for PySystemService {
    fn as_ref(&self) -> &SystemService {
        &self.0
    }
}

impl From<SystemService> for PySystemService {
    fn from(value: SystemService) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemService {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<SystemService, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, SystemService>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "SystemServiceDiff")]
#[derive(Clone)]
pub struct PySystemServiceDiff(pub(crate) SystemServiceDiff);

impl AsRef<SystemServiceDiff> for PySystemServiceDiff {
    fn as_ref(&self) -> &SystemServiceDiff {
        &self.0
    }
}

impl From<SystemServiceDiff> for PySystemServiceDiff {
    fn from(value: SystemServiceDiff) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PySystemServiceDiff {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<SystemServiceDiff, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, SystemServiceDiff>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffedSystemService")]
#[derive(Clone)]
pub struct PyDiffedSystemService(pub(crate) DiffedSystemService);

impl AsRef<DiffedSystemService> for PyDiffedSystemService {
    fn as_ref(&self) -> &DiffedSystemService {
        &self.0
    }
}

impl From<DiffedSystemService> for PyDiffedSystemService {
    fn from(value: DiffedSystemService) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffedSystemService {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffedSystemService, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffedSystemService>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "DiffSource")]
#[derive(Clone)]
pub struct PyDiffSource(pub(crate) DiffSource);

impl AsRef<DiffSource> for PyDiffSource {
    fn as_ref(&self) -> &DiffSource {
        &self.0
    }
}

impl From<DiffSource> for PyDiffSource {
    fn from(value: DiffSource) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyDiffSource {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<DiffSource, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, DiffSource>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "FuzzResult")]
#[derive(Clone)]
pub struct PyFuzzResult(pub(crate) FuzzResult);

impl AsRef<FuzzResult> for PyFuzzResult {
    fn as_ref(&self) -> &FuzzResult {
        &self.0
    }
}

impl From<FuzzResult> for PyFuzzResult {
    fn from(value: FuzzResult) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyFuzzResult {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<FuzzResult, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, FuzzResult>(self, py)
    }
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
