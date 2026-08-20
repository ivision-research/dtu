use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use std::str::FromStr;

use std::sync::Arc;
use std::time::Duration;

use dtu::analysis::taint::{
    MethodTaint, Origin, TaintAnalyzer, TaintAnalyzerOptions, TaintReport, TaintSeedOptions,
    TaintSeeds, TaintSink, TaintSinkKind, TaintSource, TaintSourceAndRoute,
};
use dtu::analysis::SsaClassLoader;
use dtu::db::graph::models::MethodId;
use dtu::tasks::TaskCanceller;
use dtu::utils::{ClassName, Denylist};
use pyo3::{prelude::*, types::PyTuple};

use crate::{
    context::PyContext,
    exception::{DtuBaseError, DtuError},
    graph::{GraphDB, PyMethodSpec},
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

impl From<PyTaintSource> for TaintSource {
    fn from(value: PyTaintSource) -> Self {
        match value {
            PyTaintSource::Param { register } => Self::Param { register },
            PyTaintSource::MethodCall {
                class_,
                method,
                args,
                ret,
            } => Self::MethodCall {
                class: class_.map(Into::into),
                method,
                args,
                ret,
            },
            PyTaintSource::Field { class_, name } => Self::Field {
                class: class_.into(),
                name,
            },
        }
    }
}

#[pymethods]
impl PyTaintSource {
    /// Parse the text form a report prints, e.g. `p1` or `*->getIntent()Landroid/content/Intent;`
    #[staticmethod]
    fn parse(value: &str) -> PyResult<Self> {
        TaintSource::from_str(value)
            .map(Self::from)
            .map_err(DtuError::mapper)
    }

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

#[pyclass(module = "dtu", name = "Origin")]
#[derive(Clone)]
pub enum PyOrigin {
    Direct(),
    /// Each chain runs from the method that was asked for to this one, naming methods by id
    CallGraph {
        chains: Vec<Vec<i32>>,
    },
}

impl From<Origin> for PyOrigin {
    fn from(value: Origin) -> Self {
        match value {
            Origin::Direct => Self::Direct(),
            Origin::CallGraph { chains } => Self::CallGraph {
                chains: chains
                    .into_iter()
                    .map(|it| it.into_iter().map(|id| id.raw()).collect())
                    .collect(),
            },
        }
    }
}

#[pymethods]
impl PyOrigin {
    fn __str__(&self) -> String {
        match self {
            Self::Direct() => String::from("direct"),
            Self::CallGraph { chains } => format!("call graph, {} chain(s)", chains.len()),
        }
    }

    fn __repr__(&self) -> String {
        format!("Origin({})", self.__str__())
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

    /// Why the method was analyzed
    #[getter]
    fn origin(&self) -> PyOrigin {
        PyOrigin::from(self.0.origin.clone())
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

/// Where the call graph may look for seeds beyond the methods asked for
#[pyclass(module = "dtu", name = "TaintSeedOptions")]
#[derive(Clone, Default)]
pub struct PyTaintSeedOptions {
    /// Restrict target lookups to these graph sources, empty for any
    #[pyo3(get, set)]
    pub sources: Vec<String>,
    /// Never seed methods in these classes, even when the graph reaches them
    #[pyo3(get, set)]
    pub deny_classes: Vec<PyClassName>,
}

#[pymethods]
impl PyTaintSeedOptions {
    #[new]
    #[pyo3(signature = (sources = Vec::new(), deny_classes = Vec::new()))]
    fn new(sources: Vec<String>, deny_classes: Vec<PyClassName>) -> Self {
        Self {
            sources,
            deny_classes,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "TaintSeedOptions(sources={:?}, deny_classes={})",
            self.sources,
            self.deny_classes.len()
        )
    }
}

impl From<PyTaintSeedOptions> for TaintSeedOptions {
    fn from(value: PyTaintSeedOptions) -> Self {
        let deny_classes = if value.deny_classes.is_empty() {
            None
        } else {
            let mut deny = Denylist::new();
            deny.extend(value.deny_classes.into_iter().map(ClassName::from));
            Some(deny)
        };
        Self {
            sources: value.sources,
            deny_classes,
        }
    }
}

/// How to run the analysis
#[pyclass(module = "dtu", name = "TaintOptions")]
#[derive(Clone, Default)]
pub struct PyTaintOptions {
    #[pyo3(get, set)]
    pub num_threads: Option<usize>,
    /// Maximum call depth, capped by the analyzer itself
    #[pyo3(get, set)]
    pub depth: Option<usize>,
    /// None to only analyze the methods that were asked for
    #[pyo3(get, set)]
    pub seed: Option<PyTaintSeedOptions>,
}

#[pymethods]
impl PyTaintOptions {
    #[new]
    #[pyo3(signature = (num_threads = None, depth = None, seed = None))]
    fn new(
        num_threads: Option<usize>,
        depth: Option<usize>,
        seed: Option<PyTaintSeedOptions>,
    ) -> Self {
        Self {
            num_threads,
            depth,
            seed,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "TaintOptions(num_threads={:?}, depth={:?}, seed={})",
            self.num_threads,
            self.depth,
            self.seed.is_some()
        )
    }
}

impl From<PyTaintOptions> for TaintAnalyzerOptions {
    fn from(value: PyTaintOptions) -> Self {
        let mut opts = TaintAnalyzerOptions::default();
        if let Some(threads) = value.num_threads {
            opts.num_threads = threads;
        }
        if let Some(depth) = value.depth {
            opts.set_depth(depth);
        }
        opts.seed = value.seed.map(Into::into);
        opts
    }
}

/// The same list for every method, or one list per method id
#[derive(FromPyObject)]
pub enum PySeeds {
    PerMethod(HashMap<i32, Vec<PyTaintSource>>),
    Shared(Vec<PyTaintSource>),
}

fn to_sources(sources: Vec<PyTaintSource>) -> Vec<TaintSource> {
    sources.into_iter().map(TaintSource::from).collect()
}

/// Run the taint analysis over `methods`
///
/// `seeds` is either one list applied to every method, or a dict keyed by method id. A
/// [TaintSource::Param] only means something against the signature it was written for, so a
/// register seed belongs in the dict form.
#[pyfunction]
#[pyo3(signature = (gdb, methods, seeds, *, ctx = None, options = None))]
pub fn run_taint_analysis(
    py: Python<'_>,
    gdb: &GraphDB,
    methods: Vec<PyMethodSpec>,
    seeds: PySeeds,
    ctx: Option<&PyContext>,
    options: Option<PyTaintOptions>,
) -> PyResult<PyTaintReport> {
    let owned_ctx;
    let ctx: &dyn dtu::Context = match ctx {
        Some(v) => v,
        None => {
            owned_ctx = PyContext::default();
            &owned_ctx
        }
    };

    let loader = Arc::new(
        SsaClassLoader::new(ctx)
            .ok_or_else(|| DtuError::new_err("failed to open the SSA class cache"))?,
    );

    let shared;
    let per_method;
    let seeds: &dyn TaintSeeds = match seeds {
        PySeeds::Shared(v) => {
            shared = to_sources(v);
            &shared
        }
        PySeeds::PerMethod(v) => {
            per_method = v
                .into_iter()
                .map(|(id, sources)| (MethodId::new(id), to_sources(sources)))
                .collect::<HashMap<_, _>>();
            &per_method
        }
    };

    let methods = methods
        .into_iter()
        .map(|it| it.as_ref().clone())
        .collect::<Vec<_>>();

    let (mut canceller, cancel) = TaskCanceller::new();
    let opts = TaintAnalyzerOptions::from(options.unwrap_or_default());
    let mut analyzer = TaintAnalyzer::new(ctx, &**gdb, cancel, opts, seeds, loader);

    // The analysis polls its cancel check from its own threads, but only the main thread ever
    // sees a signal, so it runs beside us and we do the watching.
    let mut interrupted: Option<PyErr> = None;
    let joined = std::thread::scope(|scope| {
        let handle = scope.spawn(|| analyzer.run(methods));
        while !handle.is_finished() {
            py.detach(|| std::thread::sleep(Duration::from_millis(100)));
            if let Err(e) = py.check_signals() {
                // Keep waiting: the run stops between methods, it does not stop here
                interrupted.get_or_insert(e);
                canceller.cancel();
            }
        }
        handle.join()
    });

    if let Some(e) = interrupted {
        return Err(e);
    }

    let (report, failed) = joined.map_err(|_| DtuError::new_err("the analysis thread panicked"))?;
    if !failed.is_empty() {
        eprintln!("{} methods failed to analyze", failed.len());
    }
    Ok(PyTaintReport::from(report))
}
