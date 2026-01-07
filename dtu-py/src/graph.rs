use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
};

use dtu::{
    db::graph::{
        get_default_graphdb,
        models::{ClassSearch, MethodCallPath, MethodSearch, MethodSpec},
        ClassSpec, DefaultGraphDatabase, GraphDatabase,
    },
    utils::ClassName,
};
use pyo3::prelude::*;

use crate::{
    context::PyContext,
    exception::DtuError,
    types::{PyAccessFlag, PyClassName},
};

#[pyclass]
pub struct GraphDB(DefaultGraphDatabase);

struct GraphError(dtu::db::graph::Error);

impl From<dtu::db::graph::Error> for GraphError {
    fn from(value: dtu::db::graph::Error) -> Self {
        Self(value)
    }
}

impl From<GraphError> for PyErr {
    fn from(value: GraphError) -> Self {
        DtuError::new_err(value.0.to_string())
    }
}

type Result<T> = std::result::Result<T, GraphError>;

/// GraphDB represents a read only view of the populated graph database
#[pymethods]
impl GraphDB {
    /// Get a new instance of the default graph database implementation
    #[new]
    fn new(pctx: &PyContext) -> Result<Self> {
        let gdb = get_default_graphdb(pctx)?;
        Ok(Self(gdb))
    }

    /// Get a set of all sources in the database
    fn get_all_sources(&self) -> Result<HashSet<String>> {
        Ok(self.0.get_all_sources()?)
    }

    /// Find all child classes of the given parent class
    #[pyo3(signature = (parent, *, parent_source = None, child_source = None))]
    fn find_child_classes_of(
        &self,
        parent: &str,
        parent_source: Option<&str>,
        child_source: Option<&str>,
    ) -> Result<Vec<PyClassSpec>> {
        let parent_class = ClassName::from(parent);
        let class_search = ClassSearch::new(&parent_class, parent_source);
        Ok(self
            .0
            .find_child_classes_of(&class_search, child_source)?
            .into_iter()
            .map(PyClassSpec::from)
            .collect())
    }

    /// Find all classes that implement the given interface
    #[pyo3(signature = (iface, *, iface_source = None, impl_source = None))]
    fn find_classes_implementing(
        &self,
        iface: &str,
        iface_source: Option<&str>,
        impl_source: Option<&str>,
    ) -> Result<Vec<PyClassSpec>> {
        let iface = ClassName::from(iface);
        let class_search = ClassSearch::new(iface.as_ref(), iface_source);
        Ok(self
            .0
            .find_classes_implementing(&class_search, impl_source)?
            .into_iter()
            .map(PyClassSpec::from)
            .collect())
    }

    /// Find all callers of the given class up to a certain depth.
    ///
    /// At least one of `class_` or `name` is required for this search. High depth values may
    /// negatively impact performance.
    #[pyo3(signature = (*, class_ = None, name = None, signature = None, method_source = None, call_source = None, depth = 5))]
    fn find_callers(
        &self,
        class_: Option<&str>,
        name: Option<&str>,
        signature: Option<&str>,
        method_source: Option<&str>,
        call_source: Option<&str>,
        depth: usize,
    ) -> PyResult<Vec<PyMethodCallPath>> {
        let cn = class_.map(ClassName::from);
        let search = MethodSearch::new_from_opts(cn.as_ref(), name, signature, method_source)
            .map_err(|_| DtuError::new_err("at least one of `class_` or `name` required"))?;

        Ok(self
            .0
            .find_callers(&search, call_source, depth)
            .map_err(GraphError)?
            .into_iter()
            .map(PyMethodCallPath::from)
            .collect::<Vec<_>>()
            .into())
    }

    /// Find all calls leaving the given method up to a given depth.
    #[pyo3(signature = (*, class_ = None, name = None, signature = None, source = None, depth = 5))]
    fn find_outgoing_calls(
        &self,
        class_: Option<&str>,
        name: Option<&str>,
        signature: Option<&str>,
        source: Option<&str>,
        depth: usize,
    ) -> PyResult<Vec<PyMethodCallPath>> {
        let cn = class_.map(ClassName::from);
        let search = MethodSearch::new_from_opts(cn.as_ref(), name, signature, source)
            .map_err(|_| DtuError::new_err("at least one of `class_` or `name` required"))?;

        Ok(self
            .0
            .find_outgoing_calls(&search, depth)
            .map_err(GraphError)?
            .into_iter()
            .map(PyMethodCallPath::from)
            .collect())
    }

    /// Get all classes defined by the given source
    fn get_classes_for(&self, src: &str) -> Result<Vec<PyClassName>> {
        Ok(self
            .0
            .get_classes_for(src)?
            .into_iter()
            .map(PyClassName::from)
            .collect())
    }

    /// Get all methods defined by the given soruce
    fn get_methods_for(&self, source: &str) -> Result<Vec<PyMethodSpec>> {
        Ok(self
            .0
            .get_methods_for(source)?
            .into_iter()
            .map(PyMethodSpec::from)
            .collect())
    }
}

impl Deref for GraphDB {
    type Target = DefaultGraphDatabase;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GraphDB {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[pyclass(name = "ClassSpec")]
pub struct PyClassSpec(ClassSpec);

#[pymethods]
impl PyClassSpec {
    fn is_public(&self) -> bool {
        self.0.is_public()
    }

    fn is_not_abstract(&self) -> bool {
        self.0.is_not_abstract()
    }

    #[getter]
    fn name(&self) -> PyClassName {
        PyClassName::from(self.0.name.clone())
    }

    #[getter]
    fn source(&self) -> &str {
        &self.0.source
    }

    #[getter]
    fn access_flags(&self) -> PyAccessFlag {
        self.0.access_flags.into()
    }

    fn __str__(&self) -> String {
        self.0.name.to_string()
    }
}

impl From<ClassSpec> for PyClassSpec {
    fn from(value: ClassSpec) -> Self {
        Self(value)
    }
}

#[pyclass(frozen, name = "MethodSpec")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyMethodSpec(pub(crate) MethodSpec);

#[pymethods]
impl PyMethodSpec {
    #[getter]
    fn class_(&self) -> PyClassName {
        self.0.class.clone().into()
    }

    #[getter]
    fn ret(&self) -> &str {
        &self.0.ret
    }

    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn signature(&self) -> &str {
        &self.0.signature
    }

    #[getter]
    fn source(&self) -> &str {
        &self.0.source
    }

    fn __str__(&self) -> String {
        self.0.to_string()
    }
}

impl From<MethodSpec> for PyMethodSpec {
    fn from(v: MethodSpec) -> Self {
        Self(v)
    }
}

impl From<PyMethodSpec> for MethodSpec {
    fn from(v: PyMethodSpec) -> Self {
        v.0
    }
}

#[pyclass(frozen, name = "MethodCallPath")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyMethodCallPath(pub(crate) MethodCallPath);

#[pymethods]
impl PyMethodCallPath {
    #[getter]
    fn path(&self) -> Vec<PyMethodSpec> {
        self.0.path.clone().into_iter().map(Into::into).collect()
    }

    fn source(&self) -> PyResult<String> {
        Ok(self
            .0
            .path
            .first()
            .map(|it| it.source.clone())
            .ok_or_else(|| DtuError::new_err("attempted to call source on empty call path"))?)
    }

    fn initial(&self) -> PyResult<PyMethodSpec> {
        Ok(self
            .0
            .path
            .first()
            .map(|it| PyMethodSpec::from(it.clone()))
            .ok_or_else(|| DtuError::new_err("attempted to call initial on empty call path"))?)
    }
}

impl From<MethodCallPath> for PyMethodCallPath {
    fn from(v: MethodCallPath) -> Self {
        Self(v)
    }
}

impl From<PyMethodCallPath> for MethodCallPath {
    fn from(v: PyMethodCallPath) -> Self {
        v.0
    }
}
