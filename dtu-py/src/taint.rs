use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use dtu::analysis::taint::{
    MethodTaint, TaintReport, TaintSink, TaintSinkKind, TaintSource, TaintSourceAndRoute,
};
use dtu::utils::ClassName;
use pyo3::{prelude::*, types::PyTuple};

use crate::{
    exception::{DtuBaseError, DtuError},
    graph::PyMethodSpec,
    types::PyClassName,
    utils::{reduce, unpickle},
};

#[pyclass(module = "dtu", name = "TaintSource")]
#[derive(Clone)]
pub enum PyTaintSource {
    Param {
        register: u16,
    },
    MethodCall {
        /// None when the source matches any receiver type
        class_: Option<PyClassName>,
        method: String,
        args: String,
        /// None when the source matches any return type
        ret: Option<String>,
    },
    Field {
        class_: PyClassName,
        name: String,
    },
}

impl From<TaintSource> for PyTaintSource {
    fn from(value: TaintSource) -> Self {
        match value {
            TaintSource::Param { register } => Self::Param { register },
            TaintSource::MethodCall {
                class,
                method,
                args,
                ret,
            } => Self::MethodCall {
                class_: class.map(Into::into),
                method,
                args,
                ret,
            },
            TaintSource::Field { class, name } => Self::Field {
                class_: class.into(),
                name,
            },
        }
    }
}

#[pymethods]
impl PyTaintSource {
    fn __str__(&self) -> String {
        match self {
            Self::Param { register } => format!("p{register}"),
            Self::Field { class_, name } => {
                format!("{}->{name}", AsRef::<ClassName>::as_ref(class_))
            }
            Self::MethodCall {
                class_,
                method,
                args,
                ret,
            } => {
                let ret = ret.as_deref().unwrap_or("");
                match class_ {
                    Some(class_) => format!(
                        "{}->{method}({args}){ret}",
                        AsRef::<ClassName>::as_ref(class_)
                    ),
                    None => format!("*->{method}({args}){ret}"),
                }
            }
        }
    }

    fn __repr__(&self) -> String {
        format!("TaintSource({})", self.__str__())
    }
}

#[pyclass(module = "dtu", name = "TaintSinkKind")]
#[derive(Clone)]
pub enum PyTaintSinkKind {
    Phi(),
    Instruction {
        opcode: String,
    },
    Array(),
    Field {
        class_: PyClassName,
        name: String,
    },
    MethodCall {
        method: i32,
    },
    ExternalCall {
        class_: PyClassName,
        name: String,
        args: String,
    },
}

impl From<TaintSinkKind> for PyTaintSinkKind {
    fn from(value: TaintSinkKind) -> Self {
        match value {
            TaintSinkKind::Phi => Self::Phi(),
            TaintSinkKind::Array => Self::Array(),
            TaintSinkKind::Instruction { opcode } => Self::Instruction { opcode },
            TaintSinkKind::Field { class, name } => Self::Field {
                class_: class.into(),
                name,
            },
            TaintSinkKind::MethodCall { method } => Self::MethodCall {
                method: method.raw(),
            },
            TaintSinkKind::ExternalCall { class, name, args } => Self::ExternalCall {
                class_: class.into(),
                name,
                args,
            },
        }
    }
}

#[pymethods]
impl PyTaintSinkKind {
    fn __str__(&self) -> String {
        match self {
            Self::Phi() => String::from("phi"),
            Self::Array() => String::from("array"),
            Self::Instruction { opcode } => opcode.clone(),
            Self::Field { class_, name } => {
                format!("{}->{name}", AsRef::<ClassName>::as_ref(class_))
            }
            Self::MethodCall { method } => format!("method {method}"),
            Self::ExternalCall { class_, name, args } => {
                format!("{}->{name}({args})", AsRef::<ClassName>::as_ref(class_))
            }
        }
    }

    fn __repr__(&self) -> String {
        format!("TaintSinkKind({})", self.__str__())
    }
}

#[pyclass(module = "dtu", frozen, name = "TaintSink")]
#[derive(Clone)]
pub struct PyTaintSink(TaintSink);

impl AsRef<TaintSink> for PyTaintSink {
    fn as_ref(&self) -> &TaintSink {
        &self.0
    }
}

impl From<TaintSink> for PyTaintSink {
    fn from(value: TaintSink) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyTaintSink {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<TaintSink, _>(value)
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, TaintSink>(self, py)
    }

    /// The id of the method the sink occurred in
    #[getter]
    fn location(&self) -> i32 {
        self.0.location.raw()
    }

    #[getter]
    fn kind(&self) -> PyTaintSinkKind {
        self.0.kind.clone().into()
    }

    fn __repr__(&self) -> String {
        format!(
            "TaintSink(location={}, kind={})",
            self.0.location.raw(),
            self.kind().__str__()
        )
    }
}

#[pyclass(module = "dtu", frozen, name = "SourceTaint")]
#[derive(Clone)]
pub struct PyTaintSourceAndRoute(TaintSourceAndRoute);

impl AsRef<TaintSourceAndRoute> for PyTaintSourceAndRoute {
    fn as_ref(&self) -> &TaintSourceAndRoute {
        &self.0
    }
}

impl From<TaintSourceAndRoute> for PyTaintSourceAndRoute {
    fn from(value: TaintSourceAndRoute) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyTaintSourceAndRoute {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<TaintSourceAndRoute, _>(value)
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, TaintSourceAndRoute>(self, py)
    }

    #[getter]
    fn source(&self) -> PyTaintSource {
        self.0.source.clone().into()
    }

    /// Each entry is one route the taint took from the source to an endpoint
    #[getter]
    fn paths(&self) -> Vec<Vec<PyTaintSink>> {
        self.0
            .paths
            .iter()
            .map(|path| path.iter().cloned().map(PyTaintSink::from).collect())
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "SourceTaint(source={}, paths={})",
            self.source().__str__(),
            self.0.paths.len()
        )
    }
}

#[pyclass(module = "dtu", frozen, name = "MethodTaint")]
#[derive(Clone)]
pub struct PyMethodTaint(MethodTaint);

impl AsRef<MethodTaint> for PyMethodTaint {
    fn as_ref(&self) -> &MethodTaint {
        &self.0
    }
}

impl From<MethodTaint> for PyMethodTaint {
    fn from(value: MethodTaint) -> Self {
        Self(value)
    }
}

#[pymethods]
impl PyMethodTaint {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<MethodTaint, _>(value)
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, MethodTaint>(self, py)
    }

    /// The id of the method the taint was found in
    #[getter]
    fn method(&self) -> i32 {
        self.0.method.raw()
    }

    #[getter]
    fn taint(&self) -> Vec<PyTaintSourceAndRoute> {
        self.0.taint.iter().cloned().map(Into::into).collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "MethodTaint(method={}, taint={})",
            self.0.method.raw(),
            self.0.taint.len()
        )
    }
}

/// The output of a taint analysis run
#[pyclass(module = "dtu", frozen, name = "TaintReport")]
pub struct PyTaintReport {
    report: TaintReport,
    by_id: HashMap<i32, PyMethodSpec>,
}

impl AsRef<TaintReport> for PyTaintReport {
    fn as_ref(&self) -> &TaintReport {
        &self.report
    }
}

impl From<TaintReport> for PyTaintReport {
    fn from(report: TaintReport) -> Self {
        let by_id = report
            .methods
            .iter()
            .map(|(id, spec)| (id.raw(), PyMethodSpec::from(spec.clone())))
            .collect();
        Self { report, by_id }
    }
}

#[pymethods]
impl PyTaintReport {
    #[staticmethod]
    fn __unpickle(value: &[u8]) -> PyResult<Self> {
        unpickle::<TaintReport, _>(value)
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        reduce::<_, TaintReport>(self, py)
    }

    /// Parse a report from the JSON emitted by `dtu analysis receiver-intents`
    #[staticmethod]
    fn from_json(value: &str) -> Result<Self, DtuBaseError> {
        Ok(Self::from(TaintReport::from_json(value)?))
    }

    /// Parse a report from a file containing the JSON emitted by the CLI
    #[staticmethod]
    fn from_file(path: PathBuf) -> PyResult<Self> {
        let contents = fs::read_to_string(&path).map_err(DtuError::mapper)?;
        Ok(Self::from_json(&contents)?)
    }

    /// The taint found, one entry per method
    #[getter]
    fn analysis(&self) -> Vec<PyMethodTaint> {
        self.report
            .analysis
            .iter()
            .cloned()
            .map(PyMethodTaint::from)
            .collect()
    }

    /// Every method named by an id anywhere in the report
    #[getter]
    fn methods(&self) -> Vec<PyMethodSpec> {
        self.by_id.values().cloned().collect()
    }

    /// Resolve a method id, returning None if the report doesn't name it
    fn method(&self, id: i32) -> Option<PyMethodSpec> {
        self.by_id.get(&id).cloned()
    }

    fn __repr__(&self) -> String {
        format!(
            "TaintReport(analysis={}, methods={})",
            self.report.analysis.len(),
            self.by_id.len()
        )
    }
}
