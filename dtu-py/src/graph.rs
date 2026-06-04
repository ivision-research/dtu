use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
};

use dtu::{
    db::graph::{
        get_default_graphdb,
        models::{
            ClassSearch, FieldAccessOp, FieldRef, FieldSearch, FieldSpec, MethodCallPath,
            MethodSearch, MethodSpec,
        },
        ClassSpec, DefaultGraphDatabase, GraphDatabase,
    },
    utils::ClassName,
};
use pyo3::{prelude::*, types::PyTuple};

use crate::{
    context::PyContext,
    exception::DtuError,
    types::{PyAccessFlag, PyClassName},
    utils::{reduce, unpickle},
};

#[pyclass]
pub struct GraphDB(DefaultGraphDatabase);

struct GraphError(dtu::db::Error);

impl From<dtu::db::Error> for GraphError {
    fn from(value: dtu::db::Error) -> Self {
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
    #[pyo3(signature = (ctx = None))]
    fn new(ctx: Option<&PyContext>) -> Result<Self> {
        Ok(Self(match ctx {
            Some(v) => get_default_graphdb(v),
            None => get_default_graphdb(&dtu::DefaultContext::new()),
        }?))
    }

    /// Get a set of all sources in the database
    fn get_all_sources(&self) -> Result<HashSet<String>> {
        Ok(self.0.get_all_sources()?)
    }

    /// Get all methods referencing the given field
    #[pyo3(signature = (field, *, only_read = false, only_write = false))]
    fn get_methods_referencing_field(
        &self,
        field: i32,
        only_read: bool,
        only_write: bool,
    ) -> Result<Vec<PyMethodSpec>> {
        let action = if only_write {
            Some(FieldAccessOp::Write)
        } else if only_read {
            Some(FieldAccessOp::Read)
        } else {
            None
        };

        Ok(self
            .0
            .get_methods_referencing_field(field, action)?
            .into_iter()
            .map(PyMethodSpec::from)
            .collect())
    }

    /// Get all fields referenced by the given method
    fn get_method_field_refs(&self, method: i32) -> Result<Vec<PyFieldRef>> {
        Ok(self
            .0
            .get_method_field_refs(method)?
            .into_iter()
            .map(PyFieldRef::from)
            .collect())
    }

    /// Find all methods that contain the given constant string
    fn get_methods_for_string(&self, string: &str) -> Result<Vec<PyMethodSpec>> {
        Ok(self
            .0
            .get_methods_for_string(string)?
            .into_iter()
            .map(PyMethodSpec::from)
            .collect())
    }

    /// Find all strings in a given source
    fn get_strings_for_source(&self, source: &str) -> Result<Vec<String>> {
        Ok(self.0.get_strings_for_source(source)?)
    }

    /// Find all strings in a given method
    fn get_strings_for_method(&self, method: i32) -> Result<Vec<String>> {
        Ok(self.0.get_strings_for_method(method)?)
    }

    /// Find all parent classes of the given child class
    #[pyo3(signature = (child, source))]
    fn find_parent_classes_of(&self, child: &str, source: &str) -> Result<Vec<PyClassSpec>> {
        let class = ClassName::from(child);
        Ok(self
            .0
            .find_parent_classes_of(&class, source)?
            .into_iter()
            .map(PyClassSpec::from)
            .collect())
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

    /// Get all methods defined by the given source
    fn get_methods_for(&self, source: &str) -> Result<Vec<PyMethodSpec>> {
        Ok(self
            .0
            .get_methods_for(source)?
            .into_iter()
            .map(PyMethodSpec::from)
            .collect())
    }

    /// Find all fields matching the given parameters
    #[pyo3(signature = (class_, *, name = None, type_ = None, source = None))]
    fn get_fields(
        &self,
        class_: &str,
        name: Option<&str>,
        type_: Option<&str>,
        source: Option<&str>,
    ) -> PyResult<Vec<PyFieldSpec>> {
        let cn = ClassName::from(class_);
        let search = FieldSearch::new_from_opts(&cn, name, type_, source)
            .map_err(|_| DtuError::new_err("invalid field search"))?;
        Ok(self
            .0
            .get_fields(&search)
            .map_err(GraphError)?
            .into_iter()
            .map(PyFieldSpec::from)
            .collect::<Vec<_>>()
            .into())
    }

    /// Find all methods matching the given parameters
    ///
    /// At least one of `class_` or `name` is required for this search
    #[pyo3(signature = (*, class_ = None, name = None, signature = None, source = None))]
    fn get_methods(
        &self,
        class_: Option<&str>,
        name: Option<&str>,
        signature: Option<&str>,
        source: Option<&str>,
    ) -> PyResult<Vec<PyMethodSpec>> {
        let cn = class_.map(ClassName::from);
        let search = MethodSearch::new_from_opts(cn.as_ref(), name, signature, source)
            .map_err(|_| DtuError::new_err("at least one of `class_` or `name` required"))?;
        Ok(self
            .0
            .get_methods(&search)
            .map_err(GraphError)?
            .into_iter()
            .map(PyMethodSpec::from)
            .collect::<Vec<_>>()
            .into())
    }

    /// Get all classes defining the given method
    #[pyo3(signature = (name, *, args = None, source = None))]
    fn find_classes_with_method(
        &self,
        name: &str,
        args: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<PyClassSpec>> {
        Ok(self
            .0
            .find_classes_with_method(name, args, source)?
            .into_iter()
            .map(PyClassSpec::from)
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

#[pyclass(module = "dtu", name = "ClassSpec")]
pub struct PyClassSpec(ClassSpec);

impl AsRef<ClassSpec> for PyClassSpec {
    fn as_ref(&self) -> &ClassSpec {
        &self.0
    }
}

#[pymethods]
impl PyClassSpec {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<ClassSpec, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, ClassSpec>(self, py)
    }
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

#[pyclass(module = "dtu", frozen, name = "FieldRef")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyFieldRef(pub(crate) FieldRef);

impl AsRef<FieldRef> for PyFieldRef {
    fn as_ref(&self) -> &FieldRef {
        &self.0
    }
}

#[pymethods]
impl PyFieldRef {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<FieldRef, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, FieldRef>(self, py)
    }

    #[getter]
    fn field(&self) -> PyFieldSpec {
        self.0.field.clone().into()
    }

    #[getter]
    fn is_read(&self) -> bool {
        self.0.op.is_read()
    }

    #[getter]
    fn is_write(&self) -> bool {
        self.0.op.is_write()
    }
}

impl From<FieldRef> for PyFieldRef {
    fn from(v: FieldRef) -> Self {
        Self(v)
    }
}

impl From<PyFieldRef> for FieldRef {
    fn from(v: PyFieldRef) -> Self {
        v.0
    }
}

#[pyclass(module = "dtu", frozen, name = "FieldSpec")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyFieldSpec(pub(crate) FieldSpec);

impl AsRef<FieldSpec> for PyFieldSpec {
    fn as_ref(&self) -> &FieldSpec {
        &self.0
    }
}

#[pymethods]
impl PyFieldSpec {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<FieldSpec, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, FieldSpec>(self, py)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }

    #[getter]
    fn class_(&self) -> PyClassName {
        self.0.class.clone().into()
    }

    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn ty(&self) -> &str {
        &self.0.ty
    }

    #[getter]
    fn source(&self) -> &str {
        &self.0.source
    }

    fn __str__(&self) -> String {
        self.0.to_string()
    }
}

impl From<FieldSpec> for PyFieldSpec {
    fn from(v: FieldSpec) -> Self {
        Self(v)
    }
}

impl From<PyFieldSpec> for FieldSpec {
    fn from(v: PyFieldSpec) -> Self {
        v.0
    }
}

#[pyclass(module = "dtu", frozen, name = "MethodSpec")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyMethodSpec(pub(crate) MethodSpec);

impl AsRef<MethodSpec> for PyMethodSpec {
    fn as_ref(&self) -> &MethodSpec {
        &self.0
    }
}

#[pymethods]
impl PyMethodSpec {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<MethodSpec, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, MethodSpec>(self, py)
    }
    #[getter]
    fn id(&self) -> i32 {
        self.0.id
    }

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

#[pyclass(module = "dtu", frozen, name = "MethodCallPath")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyMethodCallPath(pub(crate) MethodCallPath);

impl AsRef<MethodCallPath> for PyMethodCallPath {
    fn as_ref(&self) -> &MethodCallPath {
        &self.0
    }
}

#[pymethods]
impl PyMethodCallPath {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<MethodCallPath, _>(value)
    }
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, MethodCallPath>(self, py)
    }
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
