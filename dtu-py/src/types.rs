use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hash, Hasher},
    process::ExitStatus,
};

use dtu::{
    command::{err_on_status, CmdOutput},
    smalisa::AccessFlag,
    utils::{ClassName, DevicePath},
    UnknownBool,
};
use pyo3::prelude::*;

use crate::exception::DtuBaseError;

#[derive(Clone, PartialEq, Eq, Hash)]
#[pyclass(module = "dtu", eq, frozen, name = "DevicePath")]
pub struct PyDevicePath(DevicePath);

impl From<DevicePath> for PyDevicePath {
    fn from(value: DevicePath) -> Self {
        Self(value.clone())
    }
}

impl From<PyDevicePath> for DevicePath {
    fn from(value: PyDevicePath) -> Self {
        value.0
    }
}

impl AsRef<DevicePath> for PyDevicePath {
    fn as_ref(&self) -> &DevicePath {
        &self.0
    }
}

#[pymethods]
impl PyDevicePath {
    #[new]
    fn new(s: String) -> Self {
        Self(DevicePath::new(s))
    }

    fn __repr__(&self) -> String {
        format!("DevicePath({:?})", self.0.as_device_str())
    }

    #[staticmethod]
    fn from_squashed(s: String) -> Self {
        Self(DevicePath::from_squashed(s))
    }

    #[getter]
    fn extension(&self) -> Option<&str> {
        self.0.extension()
    }

    #[getter]
    fn device_file_name(&self) -> &str {
        self.0.device_file_name()
    }

    #[getter]
    fn device_str(&self) -> &str {
        self.0.as_device_str()
    }
}

#[pyclass(module = "dtu", frozen, eq, name = "UnknownBool")]
#[derive(Clone, Copy, PartialEq)]
pub struct PyUnknownBool(pub(crate) UnknownBool);

#[pymethods]
impl PyUnknownBool {
    #[staticmethod]
    fn unknown() -> Self {
        Self(UnknownBool::Unknown)
    }

    #[staticmethod]
    #[pyo3(name = "true")]
    fn true_() -> Self {
        Self(UnknownBool::True)
    }

    #[staticmethod]
    #[pyo3(name = "false")]
    fn false_() -> Self {
        Self(UnknownBool::False)
    }

    #[getter]
    fn is_unknown(&self) -> bool {
        matches!(self.0, UnknownBool::Unknown)
    }

    #[getter]
    fn is_true(&self) -> bool {
        matches!(self.0, UnknownBool::True)
    }

    #[getter]
    fn is_false(&self) -> bool {
        matches!(self.0, UnknownBool::False)
    }

    fn __repr__(&self) -> String {
        match self.0 {
            UnknownBool::True => "UnknownBool.TRUE".to_string(),
            UnknownBool::False => "UnknownBool.FALSE".to_string(),
            UnknownBool::Unknown => "UnknownBool.UNKNOWN".to_string(),
        }
    }

    #[classattr]
    const TRUE: Self = Self(UnknownBool::True);
    #[classattr]
    const FALSE: Self = Self(UnknownBool::False);
    #[classattr]
    const UNKNOWN: Self = Self(UnknownBool::Unknown);
}

impl From<UnknownBool> for PyUnknownBool {
    fn from(v: UnknownBool) -> Self {
        Self(v)
    }
}

impl From<PyUnknownBool> for UnknownBool {
    fn from(v: PyUnknownBool) -> Self {
        v.0
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
#[pyclass(module = "dtu", eq, frozen, name = "ClassName")]
pub struct PyClassName(ClassName);

impl From<ClassName> for PyClassName {
    fn from(value: ClassName) -> Self {
        Self(value.clone())
    }
}

impl From<PyClassName> for ClassName {
    fn from(value: PyClassName) -> Self {
        value.0
    }
}

impl AsRef<ClassName> for PyClassName {
    fn as_ref(&self) -> &ClassName {
        &self.0
    }
}

#[pymethods]
impl PyClassName {
    #[new]
    fn new(name: String) -> Self {
        Self(ClassName::new(name))
    }

    #[staticmethod]
    fn from_split_manifest(pkg: &str, name: &str) -> Self {
        Self(ClassName::from_split_manifest(pkg, name))
    }

    fn has_pkg(&self) -> bool {
        self.0.has_pkg()
    }

    fn __str__(&self) -> String {
        format!("{}", self.0)
    }

    fn __repr__(&self) -> String {
        format!("ClassName({:?})", self.0.get_java_name())
    }

    fn get_simple_class_name(&self) -> &str {
        self.0.get_simple_class_name()
    }

    fn pkg_as_java(&self) -> Cow<'_, str> {
        self.0.pkg_as_java()
    }

    fn is_java(&self) -> bool {
        self.0.is_java()
    }

    fn is_smali(&self) -> bool {
        self.0.is_smali()
    }

    fn get_java_name(&self) -> Cow<'_, str> {
        self.0.get_java_name()
    }

    fn get_smali_name(&self) -> Cow<'_, str> {
        self.0.get_smali_name()
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[pyclass(module = "dtu", name = "CmdOutput")]
#[derive(Clone)]
pub struct PyCmdOutput(CmdOutput);

impl From<CmdOutput> for PyCmdOutput {
    fn from(value: CmdOutput) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyCmdOutput {
    #[getter]
    fn status(&self) -> PyExitStatus {
        PyExitStatus(self.0.status)
    }

    #[getter]
    fn stdout(&self) -> &[u8] {
        self.0.stdout.as_slice()
    }

    #[getter]
    fn stderr(&self) -> &[u8] {
        self.0.stderr.as_slice()
    }

    fn throw_on_status(&self) -> Result<(), DtuBaseError> {
        Ok(err_on_status(self.0.status)?)
    }

    fn ok(&self) -> bool {
        self.0.ok()
    }
}

#[pyclass(module = "dtu", name = "ExitStatus")]
#[derive(Clone, Copy)]
pub struct PyExitStatus(ExitStatus);

impl From<ExitStatus> for PyExitStatus {
    fn from(value: ExitStatus) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyExitStatus {
    fn ok(&self) -> bool {
        self.0.success()
    }

    fn as_int(&self) -> Option<i32> {
        self.0.code()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[pyclass(module = "dtu", frozen, eq, hash, name = "AccessFlag")]
pub struct PyAccessFlag(pub(crate) AccessFlag);

impl From<AccessFlag> for PyAccessFlag {
    fn from(v: AccessFlag) -> Self {
        Self(v)
    }
}
impl From<PyAccessFlag> for AccessFlag {
    fn from(v: PyAccessFlag) -> Self {
        v.0
    }
}

#[pymethods]
impl PyAccessFlag {
    #[new]
    #[pyo3(signature = (value=0))]
    fn new(value: u64) -> Self {
        Self(AccessFlag::from_bits_truncate(value))
    }

    #[staticmethod]
    fn from_bits(value: u64) -> Option<Self> {
        AccessFlag::from_bits(value).map(Self)
    }

    #[staticmethod]
    fn from_bits_truncate(value: u64) -> Self {
        Self(AccessFlag::from_bits_truncate(value))
    }

    fn bits(&self) -> u64 {
        self.0.bits()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, other: &Self) -> bool {
        self.0.contains(other.0)
    }

    fn intersects(&self, other: &Self) -> bool {
        self.0.intersects(other.0)
    }

    fn union(&self, other: &Self) -> Self {
        Self(self.0 | other.0)
    }

    fn intersection(&self, other: &Self) -> Self {
        Self(self.0 & other.0)
    }

    fn difference(&self, other: &Self) -> Self {
        Self(self.0 - other.0)
    }

    fn symmetric_difference(&self, other: &Self) -> Self {
        Self(self.0 ^ other.0)
    }

    fn without(&self, other: &Self) -> Self {
        Self(self.0 & !other.0)
    }

    fn __int__(&self) -> u64 {
        self.0.bits()
    }

    fn __index__(&self) -> u64 {
        self.0.bits()
    }

    fn __repr__(&self) -> String {
        format!("AccessFlag({:#x})", self.0.bits())
    }

    fn __str__(&self) -> String {
        format!("{:?}", self.0)
    }

    fn __or__(&self, rhs: &Self) -> Self {
        Self(self.0 | rhs.0)
    }

    fn __and__(&self, rhs: &Self) -> Self {
        Self(self.0 & rhs.0)
    }

    fn __xor__(&self, rhs: &Self) -> Self {
        Self(self.0 ^ rhs.0)
    }

    #[classattr]
    const UNSET: Self = Self(AccessFlag::UNSET);
    #[classattr]
    const PUBLIC: Self = Self(AccessFlag::PUBLIC);
    #[classattr]
    const PRIVATE: Self = Self(AccessFlag::PRIVATE);
    #[classattr]
    const PROTECTED: Self = Self(AccessFlag::PROTECTED);
    #[classattr]
    const STATIC: Self = Self(AccessFlag::STATIC);
    #[classattr]
    const FINAL: Self = Self(AccessFlag::FINAL);
    #[classattr]
    const SYNCHRONIZED: Self = Self(AccessFlag::SYNCHRONIZED);
    #[classattr]
    const BRIDGE: Self = Self(AccessFlag::BRIDGE);
    #[classattr]
    const VARARGS: Self = Self(AccessFlag::VARARGS);
    #[classattr]
    const NATIVE: Self = Self(AccessFlag::NATIVE);
    #[classattr]
    const ABSTRACT: Self = Self(AccessFlag::ABSTRACT);
    #[classattr]
    const STRICTFP: Self = Self(AccessFlag::STRICTFP);
    #[classattr]
    const SYNTHETIC: Self = Self(AccessFlag::SYNTHETIC);
    #[classattr]
    const CONSTRUCTOR: Self = Self(AccessFlag::CONSTRUCTOR);
    #[classattr]
    const DECLARED_SYNCHRONIZED: Self = Self(AccessFlag::DECLARED_SYNCHRONIZED);
    #[classattr]
    const INTERFACE: Self = Self(AccessFlag::INTERFACE);
    #[classattr]
    const ENUM: Self = Self(AccessFlag::ENUM);
    #[classattr]
    const ANNOTATION: Self = Self(AccessFlag::ANNOTATION);
    #[classattr]
    const VOLATILE: Self = Self(AccessFlag::VOLATILE);
    #[classattr]
    const TRANSIENT: Self = Self(AccessFlag::TRANSIENT);

    #[classattr]
    const WHITELIST: Self = Self(AccessFlag::WHITELIST);
    #[classattr]
    const GREYLIST: Self = Self(AccessFlag::GREYLIST);
    #[classattr]
    const BLACKLIST: Self = Self(AccessFlag::BLACKLIST);
    #[classattr]
    const GREYLIST_MAX_O: Self = Self(AccessFlag::GREYLIST_MAX_O);
    #[classattr]
    const GREYLIST_MAX_P: Self = Self(AccessFlag::GREYLIST_MAX_P);
    #[classattr]
    const GREYLIST_MAX_Q: Self = Self(AccessFlag::GREYLIST_MAX_Q);
    #[classattr]
    const GREYLIST_MAX_R: Self = Self(AccessFlag::GREYLIST_MAX_R);
    #[classattr]
    const GREYLIST_MAX_S: Self = Self(AccessFlag::GREYLIST_MAX_S);
    #[classattr]
    const GREYLIST_MAX_T: Self = Self(AccessFlag::GREYLIST_MAX_T);
    #[classattr]
    const GREYLIST_MAX_U: Self = Self(AccessFlag::GREYLIST_MAX_U);
    #[classattr]
    const GREYLIST_MAX_V: Self = Self(AccessFlag::GREYLIST_MAX_V);

    #[classattr]
    const ANDROID_RESTRICTIONS: Self = Self(AccessFlag::ANDROID_RESTRICTIONS);

    #[classattr]
    const CORE_PLATFORM_API: Self = Self(AccessFlag::CORE_PLATFORM_API);
    #[classattr]
    const TEST_API: Self = Self(AccessFlag::TEST_API);
}
