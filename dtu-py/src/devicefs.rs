use dtu::devicefs::{get_project_devicefs_helper, DeviceFSHelper, FindLimits, FindName, FindType};
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

use crate::{context::PyContext, exception::DtuBaseError, types::PyDevicePath};

#[pyclass(module = "dtu", name = "DeviceFS")]
pub struct PyDeviceFS(Box<dyn DeviceFSHelper>);

type Result<T> = std::result::Result<T, DtuBaseError>;

#[pyclass(module = "dtu", name = "FindType")]
#[derive(Clone, Copy)]
pub enum PyFindType {
    Any,
    File,
    Dir,
}

impl From<PyFindType> for FindType {
    fn from(value: PyFindType) -> Self {
        match value {
            PyFindType::Any => Self::Any,
            PyFindType::File => Self::File,
            PyFindType::Dir => Self::Dir,
        }
    }
}

#[pyclass(module = "dtu", name = "FindLimits")]
#[derive(Clone, Copy)]
pub struct PyFindLimits {
    pub mindepth: Option<usize>,
    pub maxdepth: Option<usize>,
}

impl From<PyFindLimits> for FindLimits {
    fn from(value: PyFindLimits) -> Self {
        Self {
            mindepth: value.mindepth,
            maxdepth: value.maxdepth,
        }
    }
}

#[pyclass(module = "dtu", name = "FindName")]
#[derive(Clone)]
pub enum PyFindName {
    /// Search by suffix: -name '*.jar'
    Suffix(String),
    /// Search by prefix: -name 'foo*'
    Prefix(String),
    /// Search for exact: -name 'bar'
    Exact(String),
    /// Search for exact case insensitive: -iname 'foo'
    CaseInsensitive(String),
}

impl<'a> From<&'a PyFindName> for FindName<'a> {
    fn from(value: &'a PyFindName) -> Self {
        match value {
            PyFindName::Suffix(s) => Self::Suffix(s),
            PyFindName::Prefix(s) => Self::Prefix(s),
            PyFindName::Exact(s) => Self::Exact(s),
            PyFindName::CaseInsensitive(s) => Self::CaseInsensitive(s),
        }
    }
}

#[pymethods]
impl PyDeviceFS {
    #[new]
    #[pyo3(signature = (ctx = None))]
    fn new(ctx: Option<&PyContext>) -> Result<Self> {
        Ok(PyDeviceFS(match ctx {
            Some(ctx) => get_project_devicefs_helper(ctx),
            None => get_project_devicefs_helper(&dtu::DefaultContext::new()),
        }?))
    }

    fn pull(&self, device: &PyDevicePath, local: &str) -> Result<()> {
        Ok(self.0.pull(device.as_ref(), local)?)
    }

    #[pyo3(signature = (dir, ty, on_found, *, limits = None, name = None))]
    fn find(
        &self,
        dir: &str,
        ty: PyFindType,
        on_found: &Bound<'_, PyAny>,
        limits: Option<PyFindLimits>,
        name: Option<PyFindName>,
    ) -> Result<()> {
        if !on_found.is_callable() {
            return Err(DtuBaseError::from(dtu::Error::new_generic(
                "on_found not callable",
            )));
        }

        let mut cb = |line: &str| -> anyhow::Result<()> {
            on_found.call1((line,))?;
            Ok(())
        };

        Ok(self.0.find(
            dir,
            ty.into(),
            limits.map(FindLimits::from),
            name.as_ref().map(FindName::from),
            &mut cb,
        )?)
    }
}
