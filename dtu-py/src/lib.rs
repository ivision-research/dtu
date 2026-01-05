use pyo3::prelude::*;

mod adb;
mod app_server;
mod context;
mod devicefs;
mod exception;
mod filestore;
mod graph;
mod intent_string;
mod parcel_string;
mod sql;
mod types;

#[pymodule(name = "dtu")]
mod pydtu {
    #[pymodule_export]
    use super::exception::DtuError;

    #[pymodule_export]
    use super::context::PyContext;

    #[pymodule_export]
    use super::types::{
        PyAccessFlag, PyClassName, PyCmdOutput, PyDevicePath, PyExitStatus, PyUnknownBool,
    };

    #[pymodule_export]
    use super::graph::{
        GraphDB, PyClassCallPath, PyClassMeta, PyClassSourceCallPath, PyMethodCallSearch,
    };

    #[pymodule_export]
    use super::parcel_string::ParcelValue;

    #[pymodule_export]
    use super::sql::{
        DeviceDB, PyActivity, PyActivityDiff, PyApk, PyApkDiff, PyApkPermission,
        PyApkWithPermissions, PyDeviceProperty, PyDiffSource, PyDiffedActivity, PyDiffedApk,
        PyDiffedPermission, PyDiffedProvider, PyDiffedReceiver, PyDiffedService,
        PyDiffedSystemService, PyDiffedSystemServiceMethod, PyFuzzResult, PyPermission,
        PyPermissionDiff, PyProtectedBroadcast, PyProvider, PyProviderDiff, PyReceiver,
        PyReceiverDiff, PyService, PyServiceDiff, PySystemService, PySystemServiceDiff,
        PySystemServiceImpl, PySystemServiceMethod, PySystemServiceMethodDiff,
        PyUnprotectedBroadcast,
    };

    #[pymodule_export]
    use super::app_server::{PyAppServer, PyCommandResult};

    #[pymodule_export]
    use super::filestore::PyFileStore;

    #[pymodule_export]
    use super::adb::PyAdb;

    #[pymodule_export]
    use super::devicefs::{PyDeviceFS, PyFindLimits, PyFindName, PyFindType};
}
