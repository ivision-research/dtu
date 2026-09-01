use std::collections::HashMap;

use std::str::FromStr;

use std::sync::Arc;
use std::time::Duration;

use dtu::analysis::db::taint::writer::RunMeta;
use dtu::analysis::db::GraphTaintAnalysisDb;
use dtu::analysis::taint::{
    TaintAnalyzer, TaintAnalyzerOptions, TaintSeedOptions, TaintSeeds, TaintSink, TaintSinkKind,
    TaintSource,
};
use dtu::analysis::SsaClassLoader;
use dtu::db::graph::models::MethodId;
use dtu::tasks::TaskCanceller;
use dtu::utils::{ClassName, Denylist};
use pyo3::prelude::*;

use crate::{
    context::PyContext,
    exception::{DtuBaseError, DtuError},
    graph::PyMethodSpec,
    types::PyClassName,
};

#[pyclass(module = "dtu", name = "TaintSource")]
#[derive(Clone)]
pub struct PyTaintAnalysisDb(GraphTaintAnalysisDb);

#[pymethods]
impl PyTaintAnalysisDb {
    #[new]
    #[pyo3(signature = (ctx, path, *))]
    fn new(ctx: &PyContext, path: &str) -> PyResult<Self> {
        let db = GraphTaintAnalysisDb::new_from_path(ctx, path)
            .map_err(|e| DtuBaseError::from(e.to_string()))?;
        Ok(Self(db))
    }
}

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
#[pyo3(signature = (db, methods, seeds, *, ctx = None, options = None))]
pub fn run_taint_analysis(
    py: Python<'_>,
    db: PyTaintAnalysisDb,
    methods: Vec<PyMethodSpec>,
    seeds: PySeeds,
    ctx: Option<&PyContext>,
    options: Option<PyTaintOptions>,
) -> PyResult<PyTaintAnalysisDb> {
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

    let meta = RunMeta::new(
        db.0.graph(),
        serde_json::to_string(&opts).map_err(|e| DtuBaseError::from(e.to_string()))?,
    )
    .map_err(|e| DtuBaseError::from(e.to_string()))?;
    let mut analyzer = TaintAnalyzer::new(ctx, cancel, opts, seeds, loader);

    // The analysis polls its cancel check from its own threads, but only the main thread ever
    // sees a signal, so it runs beside us and we do the watching.
    let mut interrupted: Option<PyErr> = None;
    let joined = std::thread::scope(|scope| {
        let handle = scope.spawn(|| analyzer.run(methods, db.0, &meta));
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

    let res = joined.map_err(|_| DtuError::new_err("the analysis thread panicked"))?;
    let db = res.map_err(|e| DtuBaseError::from(e.to_string()))?;
    Ok(PyTaintAnalysisDb(db))
}
