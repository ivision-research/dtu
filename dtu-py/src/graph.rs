use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
};

use dtu::db::graph::{
    get_default_graphdb,
    models::{ClassCallPath, ClassSourceCallPath, MethodCallSearch, MethodMeta},
    ClassMeta, DefaultGraphDatabase, GraphDatabase,
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

#[pymethods]
impl GraphDB {
    /// Get a new instance of the default graph database implementation
    #[new]
    fn new(pctx: &PyContext) -> Result<Self> {
        let gdb = get_default_graphdb(pctx)?;
        Ok(Self(gdb))
    }

    fn get_all_sources(&self) -> Result<BTreeSet<String>> {
        Ok(self.0.get_all_sources()?)
    }

    /// Find all child classes of the given parent class
    fn find_child_classes_of(
        &self,
        parent: &PyClassName,
        source: Option<&str>,
    ) -> Result<Vec<PyClassMeta>> {
        Ok(self
            .0
            .find_child_classes_of(parent.as_ref(), source)?
            .into_iter()
            .map(PyClassMeta::from)
            .collect())
    }

    /// Find all classes that implement the given interface
    fn find_classes_implementing(
        &self,
        iface: &PyClassName,
        source: Option<&str>,
    ) -> Result<Vec<PyClassMeta>> {
        Ok(self
            .0
            .find_classes_implementing(iface.as_ref(), source)?
            .into_iter()
            .map(PyClassMeta::from)
            .collect())
    }

    /// Find all callers of the given method
    ///
    /// Depth specifies the call depth, for example:
    ///
    /// - depth = 1 will only find immediate calls
    /// - depth = 2 will find calls that call something that calls the method
    ///
    /// and so on. A high depth value will make this call take a long time and
    /// generally a lot of indirection will cause noise in the output, as each
    /// method call further away you are the more the input can be transformed
    /// before the call you're interested in.
    ///
    /// Generally, I wouldn't go above depth = 3 for good results.
    fn find_callers(
        &self,
        method: &PyMethodCallSearch,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<PyClassSourceCallPath>> {
        Ok(self
            .0
            .find_callers(&method.as_borrowed(), depth, limit)?
            .into_iter()
            .map(PyClassSourceCallPath::from)
            .collect::<Vec<_>>()
            .into())
    }

    /// Find all calls leaving the given method up to a given depth.
    fn find_outgoing_calls(
        &self,
        from: &PyMethodMeta,
        source: &str,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<PyClassCallPath>> {
        Ok(self
            .0
            .find_outgoing_calls(&from.0, source, depth, limit)?
            .into_iter()
            .map(PyClassCallPath::from)
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
    fn get_methods_for(&self, source: &str) -> Result<Vec<PyMethodMeta>> {
        Ok(self
            .0
            .get_methods_for(source)?
            .into_iter()
            .map(PyMethodMeta::from)
            .collect())
    }

    /// Wipe the database
    fn wipe(&self, ctx: &PyContext) -> Result<()> {
        Ok(self.0.wipe(ctx)?)
    }

    /// Remove all references to the given source from the database
    fn remove_source(&self, source: &str) -> Result<()> {
        Ok(self.0.remove_source(source)?)
    }

    fn optimize(&self) -> Result<()> {
        Ok(self.0.optimize()?)
    }

    fn initialize(&self) -> Result<()> {
        Ok(self.0.initialize()?)
    }

    fn eval(&self, script: &str) -> Result<String> {
        let mut result = Vec::new();
        self.0.eval(script, &mut result)?;
        Ok(String::from_utf8_lossy(&result).into())
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

#[pyclass(name = "ClassMeta")]
pub struct PyClassMeta(ClassMeta);

#[pymethods]
impl PyClassMeta {
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
}

impl From<ClassMeta> for PyClassMeta {
    fn from(value: ClassMeta) -> Self {
        Self(value)
    }
}

#[pyclass(frozen, name = "MethodMeta")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyMethodMeta(pub(crate) MethodMeta);

#[pymethods]
impl PyMethodMeta {
    #[getter]
    fn class(&self) -> PyClassName {
        self.0.class.clone().into()
    }

    #[getter]
    fn ret(&self) -> Option<&str> {
        self.0.ret.as_ref().map(String::as_str)
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
    fn access_flags(&self) -> PyAccessFlag {
        self.0.access_flags.into()
    }
}

impl From<MethodMeta> for PyMethodMeta {
    fn from(v: MethodMeta) -> Self {
        Self(v)
    }
}

impl From<PyMethodMeta> for MethodMeta {
    fn from(v: PyMethodMeta) -> Self {
        v.0
    }
}

#[pyclass(frozen, name = "ClassCallPath")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyClassCallPath(pub(crate) ClassCallPath);

#[pymethods]
impl PyClassCallPath {
    #[getter]
    fn class(&self) -> PyClassName {
        self.0.class.clone().into()
    }

    #[getter]
    fn path(&self) -> Vec<PyMethodMeta> {
        self.0.path.clone().into_iter().map(Into::into).collect()
    }
}

impl From<ClassCallPath> for PyClassCallPath {
    fn from(v: ClassCallPath) -> Self {
        Self(v)
    }
}

impl From<PyClassCallPath> for ClassCallPath {
    fn from(v: PyClassCallPath) -> Self {
        v.0
    }
}

#[pyclass(frozen, name = "ClassSourceCallPath")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyClassSourceCallPath(pub(crate) ClassSourceCallPath);

#[pymethods]
impl PyClassSourceCallPath {
    #[getter]
    fn class(&self) -> PyClassName {
        self.0.class.clone().into()
    }

    #[getter]
    fn source(&self) -> &str {
        &self.0.source
    }

    #[getter]
    fn path(&self) -> Vec<PyMethodMeta> {
        self.0.path.clone().into_iter().map(Into::into).collect()
    }
}

impl From<ClassSourceCallPath> for PyClassSourceCallPath {
    fn from(v: ClassSourceCallPath) -> Self {
        Self(v)
    }
}

impl From<PyClassSourceCallPath> for ClassSourceCallPath {
    fn from(v: PyClassSourceCallPath) -> Self {
        v.0
    }
}

#[pyclass(frozen, name = "MethodCallSearch")]
#[derive(Clone)]
pub struct PyMethodCallSearch {
    target_method: String,
    target_method_sig: String,

    src_class: Option<PyClassName>,
    src_method_name: Option<String>,
    src_method_sig: Option<String>,

    target_class: Option<PyClassName>,
    source: Option<String>,
}

#[pymethods]
impl PyMethodCallSearch {
    #[new]
    #[pyo3(
        signature = (
            target_method,
            target_method_sig,
            *,
            src_class = None,
            src_method_name = None,
            src_method_sig = None,
            target_class = None,
            source = None
        )
    )]
    fn new(
        target_method: String,
        target_method_sig: String,
        src_class: Option<PyClassName>,
        src_method_name: Option<String>,
        src_method_sig: Option<String>,
        target_class: Option<PyClassName>,
        source: Option<String>,
    ) -> Self {
        Self {
            target_method,
            target_method_sig,
            src_class,
            src_method_name,
            src_method_sig,
            target_class,
            source,
        }
    }

    #[getter]
    fn target_method(&self) -> &str {
        &self.target_method
    }

    #[getter]
    fn target_method_sig(&self) -> &str {
        &self.target_method_sig
    }

    #[getter]
    fn src_class(&self) -> Option<PyClassName> {
        self.src_class.clone()
    }

    #[getter]
    fn src_method_name(&self) -> Option<&str> {
        self.src_method_name.as_deref()
    }

    #[getter]
    fn src_method_sig(&self) -> Option<&str> {
        self.src_method_sig.as_deref()
    }

    #[getter]
    fn target_class(&self) -> Option<PyClassName> {
        self.target_class.clone()
    }

    #[getter]
    fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
}

impl PyMethodCallSearch {
    pub(crate) fn as_borrowed(&self) -> MethodCallSearch<'_> {
        MethodCallSearch {
            target_method: &self.target_method,
            target_method_sig: &self.target_method_sig,
            src_class: self.src_class.as_ref().map(|c| c.as_ref()),
            src_method_name: self.src_method_name.as_deref(),
            src_method_sig: self.src_method_sig.as_deref(),
            target_class: self.target_class.as_ref().map(|c| c.as_ref()),
            source: self.source.as_deref(),
        }
    }
}
