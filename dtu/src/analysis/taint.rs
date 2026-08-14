//! Taint analysis engine
//!
//! Taint is defined by [TaintSource]s: input method parameters, the result of known function calls,
//! or field accesses. Taint propagates via some simple rules:
//!
//!  (1) If a method is called on a tainted `this`, the return of that method is tainted if it is
//!      "interesting". Interesting is defined as any class and [B or [C (arbitrarily dimension).
//!  (2) If a tainted value is passed into a function, that parameter is marked as tainted inside
//!      that function and the result of that function is tainted if "interesting" according to
//!      the same metric as (1). We follow the function call up to a maximum depth of 16 or
//!      whatever was provided by the user.
//!  (3) If a tainted parameter is passed to a hard coded known method that mutates itself, such
//!      as Intent.put, we taint the receiver (the `Intent` in that case) and do not follow the
//!      function call.
//!  (4) If a tainted value is part of a Phi, the whole Phi is tainted.
//!  (5) If a tainted value is assigned to a field, the field becomes tainted. Note that this
//!      is currently a bit brittle and will taint the field throughout the method, not just
//!      after its assignment.
//!  (6) If a tainted value is placed into an array, that array becomes tainted. The same caveats
//!      as fields in (5) apply.
//!
//! Some caveats for things that don't propagate as tainted:
//!
//!  (1) A `this` is not generally considered tainted because that would cause a significant amount
//!     of noise in the output for very little gain. Methods are already followed and return values
//!     from `this` functions maintain the path of the tainted parameter here. There are a few hard
//!     coded exceptions.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::bail;
use crossbeam::channel::bounded;
use crossbeam::select;
use rayon::ThreadPoolBuilder;
use smalisa::cfg::InstructionId;
use smalisa::instructions::{
    InvArgs, Invocation, INS_INVOKE_CUSTOM, INS_INVOKE_CUSTOM_RANGE, INS_INVOKE_STATIC,
    INS_INVOKE_STATIC_RANGE,
};
use smalisa::ssa::{PhiId, SsaMethod, Value, ValueId, ValueUser};
use smalisa::{AccessFlag, FieldRef, MethodRef, Primitive, Register, SmaliClassName, Type};

use crate::analysis::utils::get_ssa_method;
use crate::db::graph::{
    models::{FieldAccessOp, FieldSearch, FieldSearchParams, MethodCallPath, MethodId},
    GraphDatabase, MethodSearch, MethodSearchParams, FRAMEWORK_SOURCE,
};
use crate::tasks::{cancelable_recv, cancelable_send, TaskCancelCheck};
use crate::utils::{opt_deny, OptDenylist};
use crate::Context;
use crate::{analysis::SsaClassLoader, db::graph::MethodSpec, utils::ClassName};

// There is still some stuff to do in here to make this better and more efficient. If you try to run
// taint analysis on very complicated method it will probably blow up in your face. A few things
// that could probably help off the top of my head:

#[derive(
    PartialEq, Eq, Hash, Ord, PartialOrd, Debug, Clone, serde::Serialize, serde::Deserialize,
)]
#[serde(tag = "type")]
pub enum TaintSource {
    /// An input parameter specified as the smali register number. For example p1 would be
    /// `Param { register: 1 }`
    Param { register: u16 },
    /// The result of a method call inside the function
    ///
    /// The class and return type can be None to match any
    MethodCall {
        #[serde(default)]
        class: Option<ClassName>,
        method: String,
        args: String,
        #[serde(default)]
        ret: Option<String>,
    },
    /// The tainted value comes out of a field read
    Field { class: ClassName, name: String },
}

impl Display for TaintSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Param { register } => write!(f, "p{register}"),
            Self::Field { class, name } => write!(f, "{class}->{name}"),
            Self::MethodCall {
                class,
                method,
                args,
                ret,
            } => {
                match class {
                    Some(class) => write!(f, "{class}->{method}({args})")?,
                    None => write!(f, "*->{method}({args})")?,
                }
                match ret {
                    Some(ret) => write!(f, "{ret}"),
                    None => Ok(()),
                }
            }
        }
    }
}

/// Parses what [TaintSource]'s [Display] prints
///
/// - `p1` a parameter register
/// - `class->name(args)` the result of a call, `*` for any receiver type
/// - `class->name(args)ret` the same, narrowed to one return type
/// - `class->name` a field read
impl FromStr for TaintSource {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        if let Some(reg) = s.strip_prefix('p') {
            if let Ok(register) = reg.parse::<u16>() {
                return Ok(Self::Param { register });
            }
        }

        let Some((class, member)) = s.split_once("->") else {
            return Err(format!(
                "expected pN, class->name or class->name(args), got {s:?}"
            ));
        };
        if class.is_empty() || member.is_empty() {
            return Err(format!("both sides of -> are required in {s:?}"));
        }

        let Some((method, rest)) = member.split_once('(') else {
            return Ok(Self::Field {
                class: ClassName::from(class),
                name: member.into(),
            });
        };
        // Type descriptors never contain ')', so the first one closes the arguments
        let Some((args, ret)) = rest.split_once(')') else {
            return Err(format!("unclosed argument list in {s:?}"));
        };

        Ok(Self::MethodCall {
            // A bare * matches whatever the call site names the receiver
            class: (class != "*").then(|| ClassName::from(class)),
            method: method.into(),
            args: args.into(),
            ret: (!ret.is_empty()).then(|| ret.to_string()),
        })
    }
}

/// How to find methods to analyze beyond the ones asked for
///
/// Only affects seeding. A [TaintSource] matched inside a method being walked is matched on the
/// [MethodRef] in the instruction, which carries no source, so neither of these applies there.
#[derive(Clone, Default, Debug)]
pub struct TaintSeedOptions {
    /// Restrict target lookups to these sources, empty for any
    pub sources: Vec<String>,
    /// Never seed methods in these classes, even when the graph reaches them
    pub deny_classes: OptDenylist<ClassName>,
}

#[derive(Clone)]
pub struct TaintAnalyzerOptions {
    pub num_threads: usize,
    /// None to only analyze the methods that were asked for
    pub seed: Option<TaintSeedOptions>,
    depth: usize,
}

impl Default for TaintAnalyzerOptions {
    fn default() -> Self {
        let num_threads = rayon::max_num_threads().min(4);
        Self {
            num_threads,
            seed: None,
            depth: Self::MAX_CALL_DEPTH,
        }
    }
}

impl TaintAnalyzerOptions {
    const MAX_CALL_DEPTH: usize = 16;

    pub fn set_depth(&mut self, depth: usize) {
        if depth > Self::MAX_CALL_DEPTH {
            log::warn!("ignoring depth larger than {}", Self::MAX_CALL_DEPTH);
        } else if depth > 0 {
            self.depth = depth;
        }
    }
}

#[derive(Clone, Copy)]
struct CallData {
    /// If the function was a call that had a receiver, this will be the ValueId for the
    /// receiver, otherwise it will be none
    receiver: Option<ValueId>,
    /// Whether this is a special cased mutator
    special_mutator: bool,
    /// Whether the return value is "interesting" according to our hueristic
    returns_interesting: bool,
    /// Whether the receiver is the tainted value or not
    tainted_receiver: bool,
    /// Whether it's a call to <init>
    is_init: bool,
}

impl CallData {
    fn new(
        method: &MethodRef,
        state: &RunState,
        ins_id: InstructionId,
        inv: &Invocation,
        value: ValueId,
    ) -> Self {
        // A call with a tainted receiver or parameter taints its return value, but only when that
        // value is "interesting", this means objects and [B or [C primitive arrays.
        //
        // This could add a lot of noise including non-receivers, but one reason we have to include
        // them is something like `Intent.parseUri`, we definitely want the returned Intent to be
        // tainted in that case! We could say "receiver tainted or static calls" but without some
        // more thought about this just include everything.
        let returns_interesting = match method.return_type {
            Type::Primitive(Primitive::Byte | Primitive::Char, depth) if depth > 0 => true,
            Type::Class(_, _) => true,
            _ => false,
        };

        let special_mutator = mutates_receiver(method.class, method.name);

        let ins = inv.instruction();
        let is_receiverless = ins == INS_INVOKE_STATIC
            || ins == INS_INVOKE_STATIC_RANGE
            || ins == INS_INVOKE_CUSTOM
            || ins == INS_INVOKE_CUSTOM_RANGE;

        let receiver = if is_receiverless {
            None
        } else {
            state.ssa_method.uses(ins_id).first().copied()
        };

        let tainted_receiver = receiver.is_some_and(|it| it == value);

        let is_init = method.name == "<init>";

        Self {
            returns_interesting,
            special_mutator,
            receiver,
            tainted_receiver,
            is_init,
        }
    }
}

/// A location reached by a tainted value
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum TaintSinkKind {
    /// The tainted value entered a Phi
    Phi,
    /// The tainted value was passed into a bare instruction, named by its opcode
    ///
    /// The opcode name rather than the [Instruction] itself: smalisa's raw bits are only stable
    /// within a major version, so they are a poor thing to persist.
    Instruction { opcode: String },
    /// The tainted value is stored in an array
    Array,
    /// The tained value is stored in a field
    Field { class: ClassName, name: String },
    /// The tainted value was passed to a method the graph database knows about. Resolve the id to
    /// get the class and signature back.
    MethodCall { method: MethodId },
    /// The tainted value was passed to a method that has no id: either a class we deliberately
    /// don't descend into, or one that isn't in the dump.
    ExternalCall {
        class: ClassName,
        name: String,
        args: String,
    },
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct TaintSink {
    /// The method the sink occurred in
    pub location: MethodId,
    pub kind: TaintSinkKind,
}

/// One route a tainted value took
///
/// The first entry is the source and the last is where it was last resolved to. Note that this
/// might not be the end of the route: routes are truncated to a maximum call depth.
pub type TaintRoute = Vec<TaintSink>;

/// The taint that spread from one [TaintSource]
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct TaintSourceAndRoute {
    pub source: TaintSource,
    pub paths: Vec<TaintRoute>,
}

/// Why a method was analyzed
///
/// The chains a [Origin::CallGraph] carries come from the call graph, not from the dataflow
/// analysis: they say the method is reachable, not that anything tainted flows along them.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum Origin {
    /// The method was asked for directly
    Direct,
    /// The call graph led here from a method that was asked for
    ///
    /// Each chain runs from the method that was asked for to this one, naming methods by id the
    /// same way sinks do. Resolve them against [TaintReport::methods].
    CallGraph { chains: Vec<Vec<MethodId>> },
}

impl Origin {
    /// Whether the method was reached through the call graph rather than asked for
    pub fn is_call_graph(&self) -> bool {
        matches!(self, Self::CallGraph { .. })
    }
}

/// The taint analysis results for a single method
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct MethodTaint {
    pub method: MethodId,
    pub origin: Origin,
    pub taint: Vec<TaintSourceAndRoute>,
}

#[derive(PartialEq, Eq, Ord, PartialOrd, Hash, Debug, Clone, Copy)]
struct TaintedValue<'a> {
    source: &'a TaintSource,
    value: ValueId,
}

impl<'a> TaintedValue<'a> {
    fn new(source: &'a TaintSource, value: ValueId) -> Self {
        Self { source, value }
    }
}

impl<'a> Display for TaintedValue<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: 0x{:x}", self.source, self.value.index())
    }
}

/// The result of a run, and everything needed to read it.
///
/// Sinks name methods by [MethodId] alone, so `methods` is what lets a consumer resolve them
/// without the graph database.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaintReport {
    pub analysis: Vec<MethodTaint>,
    pub methods: HashMap<MethodId, MethodSpec>,
}

impl TaintReport {
    pub fn from_json(value: &str) -> crate::Result<Self> {
        serde_json::from_str(value)
            .map_err(|e| crate::Error::Generic(format!("invalid taint report: {e}")))
    }

    pub fn filter_reaches<F>(self, filter: F) -> Self
    where
        F: Fn(&TaintSinkKind) -> bool,
    {
        let methods = self.methods;

        let analysis = self
            .analysis
            .into_iter()
            .filter(|res| {
                res.taint.iter().any(|src_and_route| {
                    src_and_route
                        .paths
                        .iter()
                        .any(|path| path.iter().any(|sink| filter(&sink.kind)))
                })
            })
            .collect::<Vec<_>>();

        Self { methods, analysis }
    }
}

/// Gathers the [MethodSpec] behind every [MethodId] a report mentions.
///
/// Every id written into a sink is resolved at the point it is written, so the specs are already in
/// hand and no second pass over the report is needed.
#[derive(Default)]
struct MethodCollector(HashMap<MethodId, MethodSpec>);

impl MethodCollector {
    fn new() -> Self {
        Self(HashMap::new())
    }

    fn add(&mut self, method: &MethodSpec) {
        self.0.entry(method.id).or_insert_with(|| method.clone());
    }

    fn merge(&mut self, other: Self) {
        self.0.extend(other.0);
    }

    fn into_map(self) -> HashMap<MethodId, MethodSpec> {
        self.0
    }
}

/// The state carried across the interprocedural walk.
///
/// [RunState] covers a single method; this is what outlives the recursion.
struct CallState<'c> {
    /// The methods currently being analyzed, innermost last. A call back into one of them would
    /// recurse forever, so it is treated as an endpoint.
    stack: Vec<MethodId>,
    methods: &'c mut MethodCollector,
}

/// Where a run gets the seeds for each method it analyzes.
///
/// [TaintSource::Param] names a register, which only means something relative to one signature, so
/// a run over methods with different signatures cannot share a single list. Sources that match on a
/// call site can.
pub trait TaintSeeds: Sync {
    fn sources_for(&self, method: &MethodSpec) -> &[TaintSource];
}

/// The same sources for every method, for call site matched seeds
impl TaintSeeds for Vec<TaintSource> {
    fn sources_for(&self, _: &MethodSpec) -> &[TaintSource] {
        self
    }
}

/// Seeds chosen per method, which is what [TaintSource::Param] requires
impl TaintSeeds for HashMap<MethodId, Vec<TaintSource>> {
    fn sources_for(&self, method: &MethodSpec) -> &[TaintSource] {
        self.get(&method.id).map(Vec::as_slice).unwrap_or_default()
    }
}

impl TaintSource {
    /// The search matching every method whose call this seeds on
    ///
    /// [TaintSource::Param] names a register relative to one signature, so it has no search
    fn method_search<'a>(&'a self, source: Option<&'a str>) -> Option<MethodSearch<'a>> {
        let Self::MethodCall {
            class,
            method,
            args,
            ret,
        } = self
        else {
            return None;
        };
        let param = match class {
            Some(class) => MethodSearchParams::ByFullSpec {
                class,
                name: method,
                signature: args,
            },
            None => MethodSearchParams::ByNameAndSignature {
                name: method,
                signature: args,
            },
        };
        Some(MethodSearch::new(param, source, ret.as_deref()))
    }

    /// The search matching every field this seeds on
    fn field_search<'a>(&'a self, source: Option<&'a str>) -> Option<FieldSearch<'a>> {
        let Self::Field { class, name } = self else {
            return None;
        };
        Some(FieldSearch::new(
            FieldSearchParams::ByClassAndName { class, name },
            source,
        ))
    }
}

/// What a run will actually analyze
///
/// The methods asked for, plus whatever the call graph reached when seeding is on.
#[derive(Default)]
struct WorkPlan {
    methods: Vec<MethodSpec>,
    seeds: HashMap<MethodId, Vec<TaintSource>>,
    origins: HashMap<MethodId, Origin>,
    /// Methods named by a chain, so the report can resolve the ids in one
    referenced: Vec<MethodSpec>,
}

impl WorkPlan {
    fn sources_for(&self, method: &MethodSpec) -> &[TaintSource] {
        self.seeds.get(&method.id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn origin_for(&self, method: &MethodSpec) -> Origin {
        self.origins
            .get(&method.id)
            .cloned()
            .unwrap_or(Origin::Direct)
    }

    /// Add a method the call graph reached, merging into what is already planned for it
    ///
    /// A method that was asked for directly keeps that origin: it is already being analyzed with
    /// its own seeds and the chains that reach it say nothing extra.
    fn add_indirect(&mut self, method: MethodSpec, source: TaintSource, chain: MethodCallPath) {
        let ids = chain.path.iter().map(|it| it.id).collect::<Vec<MethodId>>();
        self.referenced.extend(chain.path);

        match self.origins.entry(method.id) {
            Entry::Occupied(mut existing) => {
                if let Origin::CallGraph { chains } = existing.get_mut() {
                    chains.push(ids);
                } else {
                    return;
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(Origin::CallGraph { chains: vec![ids] });
                self.methods.push(method.clone());
            }
        }
        let sources = self.seeds.entry(method.id).or_default();
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
}

/// Entrypoint for performing taint analysis
pub struct TaintAnalyzer<'a> {
    ctx: &'a dyn Context,
    db: &'a dyn GraphDatabase,
    cancel: TaskCancelCheck,
    opts: TaintAnalyzerOptions,
    loader: Arc<SsaClassLoader>,
    seeds: &'a dyn TaintSeeds,
}

struct TaintAnalyzerWorker<'a> {
    ctx: &'a dyn Context,
    db: &'a dyn GraphDatabase,
    max_depth: usize,
    cancel: &'a TaskCancelCheck,
    loader: Arc<SsaClassLoader>,
}

impl<'a> TaintAnalyzer<'a> {
    pub fn new(
        ctx: &'a dyn Context,
        db: &'a dyn GraphDatabase,
        cancel: TaskCancelCheck,
        opts: TaintAnalyzerOptions,
        seeds: &'a dyn TaintSeeds,
        loader: Arc<SsaClassLoader>,
    ) -> Self {
        Self {
            ctx,
            db,
            cancel,
            opts,
            seeds,
            loader,
        }
    }

    /// Work out everything that will be analyzed, before any of it starts
    fn plan(&self, methods: Vec<MethodSpec>) -> WorkPlan {
        let mut plan = WorkPlan::default();
        for method in &methods {
            plan.seeds
                .insert(method.id, self.seeds.sources_for(method).to_vec());
            plan.origins.insert(method.id, Origin::Direct);
        }
        plan.methods = methods;

        let Some(opts) = self.opts.seed.as_ref() else {
            return plan;
        };

        let entries = plan.methods.iter().map(|it| it.id).collect::<Vec<_>>();
        let sources = plan
            .seeds
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>();

        for source in sources {
            for chain in self.reaching_chains(&source, &entries, opts) {
                let Some(method) = chain.path.last().cloned() else {
                    continue;
                };
                if opt_deny(&opts.deny_classes, &method.class) {
                    continue;
                }
                plan.add_indirect(method, source.clone(), chain);
            }
        }

        plan
    }

    /// Every chain from `entries` to a method this source would seed on
    ///
    /// One query per allowed source, since a target can only be narrowed to one at a time.
    fn reaching_chains(
        &self,
        source: &TaintSource,
        entries: &[MethodId],
        opts: &TaintSeedOptions,
    ) -> Vec<MethodCallPath> {
        let scopes: Vec<Option<&str>> = if opts.sources.is_empty() {
            vec![None]
        } else {
            opts.sources.iter().map(|it| Some(it.as_str())).collect()
        };

        let mut found = Vec::new();
        for scope in scopes {
            let res = if let Some(search) = source.method_search(scope) {
                self.db.find_callers_from(&search, entries)
            } else if let Some(search) = source.field_search(scope) {
                self.db
                    .find_field_refs_from(&search, FieldAccessOp::Read, entries)
            } else {
                // A Param only means something against the signature it was given for
                continue;
            };
            match res {
                Ok(chains) => found.extend(chains),
                Err(e) => log::warn!("failed to expand seeds for {source}: {e}"),
            }
        }
        found
    }

    /// Run the analysis
    pub fn run(&mut self, methods: Vec<MethodSpec>) -> (TaintReport, Vec<MethodSpec>) {
        let plan = self.plan(methods);
        log::info!("Running on {} methods", plan.methods.len());

        let worker_pool = ThreadPoolBuilder::new()
            .num_threads(self.opts.num_threads.max(1))
            .build()
            .expect("building worker pool");

        let (work_tx, work_rx) = bounded(2 * worker_pool.current_num_threads());
        let (fail_tx, fail_rx) = bounded::<MethodSpec>(2 * worker_pool.current_num_threads());
        let (result_tx, result_rx) =
            bounded::<(MethodSpec, MethodPaths)>(2 * worker_pool.current_num_threads());
        let mut failures = Vec::new();
        let mut analysis: Vec<MethodTaint> = Vec::new();

        // The analyzed methods are named by the report too, and only this thread
        // sees them. The workers collect the callees they resolve.
        let mut collected = MethodCollector::new();
        for method in &plan.referenced {
            collected.add(method);
        }
        let mut worker_collected = Vec::new();

        thread::scope(|s| {
            s.spawn(|| {
                let mut it = plan.methods.iter().cloned();

                'outer: while let Some(method) = it.next() {
                    if self.cancel.was_cancelled() {
                        break;
                    }
                    loop {
                        select! {
                            recv(fail_rx) -> res => {
                                if let Ok(failed) = res {
                                    failures.push(failed);
                                }
                            },
                            recv(result_rx) -> res => {
                                if let Ok((method, path)) = res {
                                    // A method nothing reached says nothing
                                    if !path.is_empty() {
                                        let origin = plan.origin_for(&method);
                                        collected.add(&method);
                                        analysis.push(path.into_taint(method.id, origin));
                                    }
                                }
                            },
                            send(work_tx, method) -> _ => {
                                break;
                            },
                            default(Duration::from_millis(100)) => {
                                if self.cancel.was_cancelled() {
                                    break 'outer;
                                }
                            },
                        }
                    }
                }

                drop(work_tx);

                // Done sending, but may still have to receive
                loop {
                    select! {
                        recv(fail_rx) -> res => {
                            if let Ok(failed) = res {
                                failures.push(failed);
                            }
                        },
                        recv(result_rx) -> res => {
                            if let Ok((method, path)) = res {
                                // A method nothing reached says nothing
                                if !path.is_empty() {
                                    collected.add(&method);
                                    let origin = plan.origin_for(&method);
                                        collected.add(&method);
                                        analysis.push(path.into_taint(method.id, origin));
                                }
                            } else {
                                break;
                            }
                        },
                        default(Duration::from_millis(100)) => {
                            if self.cancel.was_cancelled() {
                                break;
                            }
                        },
                    }
                }
            });

            worker_collected = worker_pool.broadcast(|_| {
                let analyzer = TaintAnalyzerWorker {
                    ctx: self.ctx,
                    db: self.db,
                    max_depth: self.opts.depth,
                    cancel: &self.cancel,
                    loader: Arc::clone(&self.loader),
                };

                // Accumulated for the whole thread rather than per method, so
                // the callees a worker resolves are only cloned once.
                let mut collected = MethodCollector::new();

                loop {
                    let Ok(Some(method)) = cancelable_recv(analyzer.cancel, &work_rx) else {
                        break;
                    };
                    let sources = plan.sources_for(&method);
                    match analyzer.run(&method, sources, &mut collected) {
                        Err(e) => {
                            log::debug!("failed to analyze method {method}: {e}");
                            let Ok(true) = cancelable_send(analyzer.cancel, method, &fail_tx)
                            else {
                                break;
                            };
                        }
                        Ok(v) => {
                            let Ok(true) =
                                cancelable_send(analyzer.cancel, (method, v), &result_tx)
                            else {
                                break;
                            };
                        }
                    }
                }

                collected
            });

            drop(fail_tx);
            drop(result_tx);
        });

        for worker in worker_collected {
            collected.merge(worker);
        }

        let report = TaintReport {
            analysis,
            methods: collected.into_map(),
        };

        (report, failures)
    }
}

/// The routes found in one method while it is being walked
///
/// Keyed so a route can be appended to the source it came from. The report holds the list form,
/// which this becomes once the method is finished.
#[derive(Default)]
struct MethodPaths(HashMap<TaintSource, Vec<TaintRoute>>);

impl MethodPaths {
    fn add_path(&mut self, source: TaintSource, route: TaintRoute) {
        self.0.entry(source).or_default().push(route);
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn into_taint(self, method: MethodId, origin: Origin) -> MethodTaint {
        MethodTaint {
            method,
            origin,
            taint: self
                .0
                .into_iter()
                .map(|(source, paths)| TaintSourceAndRoute { source, paths })
                .collect(),
        }
    }
}

type TaintSet<'a> = HashSet<TaintedValue<'a>>;
type WorkItem<'a> = (TaintedValue<'a>, Option<PathId>);
type WorkQueue<'a> = Vec<WorkItem<'a>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct PathId(u32);

struct PathNode {
    sink: TaintSink,
    parent: Option<PathId>,
}

/// The sinks a tainted value has passed through, stored as a tree rather than a vec per path.
///
/// A value with several users forks the path, and every fork shares the prefix that led to it.
/// Holding the prefix once means extending is a single push and forking is a copy of a [PathId],
/// instead of cloning the whole history at every branch. [PathArena::materialize] walks back to the
/// root, and only runs when a finished path is recorded.
#[derive(Default)]
struct PathArena {
    nodes: Vec<PathNode>,
}

impl PathArena {
    fn push(&mut self, parent: Option<PathId>, sink: TaintSink) -> PathId {
        let id = PathId(self.nodes.len() as u32);
        self.nodes.push(PathNode { sink, parent });
        id
    }

    /// The path from the source to `from`, in the order it was walked.
    fn materialize(&self, from: Option<PathId>) -> Vec<TaintSink> {
        let mut out = Vec::new();
        let mut at = from;
        while let Some(id) = at {
            let node = &self.nodes[id.0 as usize];
            out.push(node.sink.clone());
            at = node.parent;
        }
        out.reverse();
        out
    }
}

/// Every value produced by a field read in the method, keyed by class and name.
///
/// Seeding a [TaintSource::Field] and propagating out of a field write both want the same answer,
/// and finding it by scanning is linear in the size of the method. There is no aliasing information
/// here: two objects of the same class share an entry.
type FieldReadIndex<'m> = HashMap<(&'m SmaliClassName, &'m str), Vec<ValueId>>;

struct RunState<'b, 'm> {
    path: MethodPaths,
    seen: TaintSet<'b>,
    field_reads: FieldReadIndex<'m>,
    work: WorkQueue<'b>,
    paths: PathArena,
    method: &'b MethodSpec,
    ssa_method: &'b SsaMethod<'m>,
}

impl<'b, 'm> RunState<'b, 'm> {
    fn new(method: &'b MethodSpec, ssa_method: &'b SsaMethod<'m>) -> Self {
        Self {
            method,
            ssa_method,
            path: MethodPaths::default(),
            seen: TaintSet::default(),
            field_reads: index_field_reads(ssa_method),
            work: WorkQueue::default(),
            paths: PathArena::default(),
        }
    }

    /// Record a finished path only if the walk isn't continuing past here.
    ///
    /// If taint moved on to another value the longer path will be recorded when it ends, and
    /// recording here as well would report a truncated duplicate.
    fn record_unless(&mut self, propagated: bool, source: &TaintSource, path: Option<PathId>) {
        if !propagated {
            self.record(source, path);
        }
    }

    /// Record a finished path for `source`
    fn record(&mut self, source: &TaintSource, path: Option<PathId>) {
        let sinks = self.paths.materialize(path);
        self.path.add_path(source.clone(), sinks);
    }

    /// Append a sink reached in this method
    fn extend(&mut self, path: Option<PathId>, kind: TaintSinkKind) -> PathId {
        self.paths.push(
            path,
            TaintSink {
                location: self.method.id,
                kind,
            },
        )
    }

    fn is_this(&self, value_id: ValueId) -> bool {
        if self.method.access_flags.contains(AccessFlag::STATIC) {
            return false;
        }
        let value = self.ssa_method.value(value_id);
        match value {
            Value::Param(v) => {
                // The unwrap is safe here, 0 can't overflow
                *v == self
                    .ssa_method
                    .regs
                    .variable(Register::new(true, 0).unwrap())
            }
            _ => false,
        }
    }

    /// Mark every defined value in the instruction as tainted and return whether any new values
    /// were reached
    fn taint_defs(
        &mut self,
        source: &'b TaintSource,
        ins_id: InstructionId,
        path: Option<PathId>,
    ) -> bool {
        let mut tainted = false;
        for &def in self.ssa_method.defs(ins_id) {
            let new_tv = TaintedValue::new(source, def);
            if self.seen.insert(new_tv) {
                tainted = true;
                self.work.push((new_tv, path));
            }
        }
        tainted
    }

    fn pop(&mut self) -> Option<WorkItem<'b>> {
        self.work.pop()
    }

    fn push_tainted(
        &mut self,
        source: &'b TaintSource,
        value: ValueId,
        path: Option<PathId>,
    ) -> bool {
        let tv = TaintedValue::new(source, value);
        if self.seen.insert(tv) {
            self.work.push((tv, path));
            true
        } else {
            false
        }
    }

    fn push(&mut self, source: &'b TaintSource, value: ValueId, path: Option<PathId>) {
        let tv = TaintedValue::new(source, value);
        if self.seen.insert(tv) {
            self.work.push((tv, path));
        } else {
            self.record(source, path);
        }
    }

    fn push_phi(&mut self, source: &'b TaintSource, phi: PhiId, path: Option<PathId>) {
        let path = self.extend(path, TaintSinkKind::Phi);
        let value = self.ssa_method.phi(phi).value;
        self.push(source, value, Some(path));
    }

    fn push_array(&mut self, source: &'b TaintSource, ins_id: InstructionId, path: Option<PathId>) {
        let path = self.extend(path, TaintSinkKind::Array);
        // Treat the array as a terminal by itself
        self.record(source, Some(path));

        // The array itself is always the first argument
        let Some(val) = self.ssa_method.uses(ins_id).first() else {
            return;
        };
        // Mark the array as tainted if it isn't already tainted
        let tv = TaintedValue::new(source, *val);
        if self.seen.insert(tv) {
            self.work.push((tv, Some(path)));
        }
    }

    fn push_field(&mut self, source: &'b TaintSource, inv: &Invocation<'m>, path: Option<PathId>) {
        let Some(target) = field_ref(inv) else {
            log::warn!("unexpected format for instruction: {}", inv.instruction());
            return;
        };

        let path = self.extend(
            path,
            TaintSinkKind::Field {
                class: ClassName::from(target.class),
                name: target.name.into(),
            },
        );

        // Treat a field as a terminal by itself
        self.record(source, Some(path));

        // We treat every read of the field as a source of taint. This isn't technically correct,
        // but we'd have to have dominance to know which ones actually matter. Maybe one day.
        let Some(reads) = self.field_reads.get(&(target.class, target.name)) else {
            return;
        };

        // The path was recorded above, so a read already reached some other way needs nothing
        // further and would only duplicate it.
        for &value in reads {
            let tv = TaintedValue::new(source, value);
            if self.seen.insert(tv) {
                self.work.push((tv, Some(path)));
            }
        }
    }

    /// Search through the provided [SsaMethod] for all registered [TaintSource]s and add them
    /// to a list of initial tainted values.
    fn seed(&mut self, sources: &'b [TaintSource]) -> anyhow::Result<()> {
        for source in sources {
            match source {
                TaintSource::Field { class, name } => {
                    let tmp = class.get_smali_name();
                    let smali_class = SmaliClassName::from_raw(&tmp);
                    if let Some(reads) = self.field_reads.get(&(&smali_class, name.as_str())) {
                        self.seen
                            .extend(reads.iter().map(|it| TaintedValue::new(source, *it)));
                    }
                }
                TaintSource::Param { register } => {
                    let var = self.ssa_method.variable(Register::new(true, *register)?);
                    self.seen.extend(
                        self.ssa_method
                            .values
                            .iter()
                            .enumerate()
                            .filter(|(_, def)| matches!(def, Value::Param(v) if *v == var))
                            .map(|(idx, _)| TaintedValue::new(source, ValueId::new(idx))),
                    );
                }
                TaintSource::MethodCall {
                    class,
                    method: name,
                    args,
                    ret,
                } => {
                    for block in self.ssa_method.reverse_post_order() {
                        for ins in self.ssa_method.block_instructions(block) {
                            let inv = self.ssa_method.instruction(ins);
                            if !inv.is_call() {
                                continue;
                            }
                            let Some(target) = call_target(inv) else {
                                continue;
                            };
                            let class_matches = class
                                .as_ref()
                                .is_none_or(|it| target.class == it.get_smali_name());
                            let ret_matches = ret.as_ref().is_none_or(|it| {
                                target
                                    .return_type
                                    .as_smali_str()
                                    .is_some_and(|rt| rt == *it)
                            });
                            if class_matches
                                && ret_matches
                                && target.name == *name
                                && target.args == *args
                            {
                                self.seen.extend(
                                    self.ssa_method
                                        .defs(ins)
                                        .iter()
                                        .map(|it| TaintedValue::new(source, *it)),
                                )
                            }
                        }
                    }
                }
            }
        }

        for tv in &self.seen {
            self.work.push((*tv, None));
        }

        Ok(())
    }

    fn on_generic_instruction(
        &mut self,
        ins_id: InstructionId,
        tv: TaintedValue<'b>,
        path: Option<PathId>,
    ) {
        let ins = self.ssa_method.instruction(ins_id).instruction();
        let path = self.extend(
            path,
            TaintSinkKind::Instruction {
                opcode: ins.to_string(),
            },
        );

        // We'll consider all outputs from instructions with tainted input to be tained. I'm not
        // sure this is the best way to handle this but that's ok.

        // Every def forks the path, and all of them share the node just pushed
        let mut terminal = true;
        for &out in self.ssa_method.defs(ins_id) {
            let new_tv = TaintedValue::new(tv.source, out);
            if self.seen.insert(new_tv) {
                terminal = false;
                self.work.push((new_tv, Some(path)));
            }
        }

        // Either the instruction writes nothing, or every value it writes was already reached some
        // other way. Nothing left to follow.
        if terminal {
            self.record(tv.source, Some(path));
        }
    }
}

impl<'a> TaintAnalyzerWorker<'a> {
    fn run(
        &self,
        method: &MethodSpec,
        sources: &[TaintSource],
        methods: &mut MethodCollector,
    ) -> anyhow::Result<MethodPaths> {
        let mut call = CallState {
            stack: Vec::new(),
            methods,
        };
        self.run_inner(method, sources, &mut call)
    }

    fn run_inner(
        &self,
        method: &MethodSpec,
        sources: &[TaintSource],
        call: &mut CallState<'_>,
    ) -> anyhow::Result<MethodPaths> {
        let class =
            self.loader
                .get_ssa_class(self.ctx, method.class_id, &method.class, &method.source)?;
        let class = class.get();

        let Some(ssa_method) = get_ssa_method(class, method) else {
            bail!("failed to find method {}", method);
        };

        let mut state = RunState::new(method, &ssa_method);
        state.seed(sources)?;

        while let Some((tv, tv_path)) = state.pop() {
            let users = ssa_method.users(tv.value);

            // Nothing consumes this value, so the path ends here
            if users.is_empty() {
                state.record(tv.source, tv_path);
                continue;
            }

            self.on_users(&mut state, users, tv, tv_path, call);
        }

        Ok(state.path)
    }

    /// Follow a tainted value into every one of its users.
    fn on_users<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        users: &'_ [ValueUser],
        tv: TaintedValue<'b>,
        path: Option<PathId>,
        call: &mut CallState<'_>,
    ) {
        for &user in users {
            match user {
                ValueUser::Phi(phi_id) => {
                    state.push_phi(tv.source, phi_id, path);
                }
                ValueUser::Instruction(ins_id) => {
                    let inv = state.ssa_method.instruction(ins_id);
                    if inv.is_call() {
                        self.on_call(state, ins_id, inv, tv, path, call);
                        continue;
                    }

                    if inv.sets_field() {
                        state.push_field(tv.source, inv, path);
                        continue;
                    }

                    if inv.sets_array_element() {
                        state.push_array(tv.source, ins_id, path);
                        continue;
                    }

                    state.on_generic_instruction(ins_id, tv, path);
                }
            }
        }
    }

    fn on_call<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        ins_id: InstructionId,
        inv: &Invocation,
        tv: TaintedValue<'b>,
        path: Option<PathId>,
        call: &mut CallState<'_>,
    ) {
        let Some(target) = call_target(inv) else {
            log::warn!("instruction {} returned no call target", inv.instruction());
            state.record(tv.source, path);
            return;
        };

        let call_data = CallData::new(target, state, ins_id, inv, tv.value);

        // This case is subtler than it looks. We don't want `intent.setFlags` to be a reported
        // call, but `Intent.setFlags` returns the `Intent` itself. This means you could have a
        // fluent call: `val newIntent = Intent().put(key, TAINTED).setFlags(10)` and if we weren't
        // careful, `setFlags` would _clear_ the taint that was applied by `put(key, TAINTED)`
        // immediately before. Instead, let's say that in that case we don't care about the call to
        // setFlags, but we do want the new return value to still be considered tainted.
        if call_data.special_mutator && call_data.tainted_receiver {
            // We want to make sure our "interesting" hueristic is observed here
            if call_data.returns_interesting {
                // Push the original path because we don't care about the call
                state.taint_defs(tv.source, ins_id, path);
            }
            // Return no matter what happens because we never recurse into these types of calls
            return;
        }

        // Resolve the callee before recording anything, so the sink can name it by id. A call we
        // can't resolve gets recorded as external instead: there is no id for a method that isn't
        // in the graph database.
        let methods = self.resolve_callee(target);
        let callee = methods
            .as_ref()
            .and_then(|it| self.pick_callee(it, state, target));

        // The sink names the callee by id, so this is where the report learns what that id refers
        // to.
        if let Some(method) = callee {
            call.methods.add(method);
        }

        // Extend the path to include this call unconditially
        let call_path = state.extend(
            path,
            match callee {
                Some(method) => TaintSinkKind::MethodCall { method: method.id },
                None => TaintSinkKind::ExternalCall {
                    class: ClassName::from(target.class),
                    name: target.name.into(),
                    args: target.args.into(),
                },
            },
        );

        let propagated =
            self.propagate(state, ins_id, tv, Some(call_path), call_data, callee, call);
        state.record_unless(propagated, tv.source, Some(call_path));
    }

    fn propagate<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        ins_id: InstructionId,
        tv: TaintedValue<'b>,
        path: Option<PathId>,
        call_data: CallData,
        callee: Option<&MethodSpec>,
        call: &mut CallState<'_>,
    ) -> bool {
        let mut propagated =
            call_data.returns_interesting && state.taint_defs(tv.source, ins_id, path);

        // Handle the case where the call should taint the receiver's value. We do this for <init>
        // and special cased receivers that are invoked fluently
        if call_data.is_init || call_data.special_mutator {
            if let Some(receiver) = call_data.receiver {
                if receiver != tv.value && !state.is_this(receiver) {
                    propagated |= state.push_tainted(tv.source, receiver, path);
                }
            }
        }

        // Special mutators are terminal: we don't want to do anything else with them, tainting the
        // receiver was enough. Note that `call_data.is_init` is NOT terminal: we are interested in
        // constructors in general.
        if call_data.special_mutator {
            return propagated;
        }

        let Some(method) = callee else {
            return propagated;
        };

        // Upon entering a new function call, if we have `p0` tainted and it is not static, we run
        // into a ton of noise because now pretty much everything becomes tainted. We don't want
        // that, so we don't taint `this` on calls.
        let has_receiver = call_data.receiver.is_some();

        let mut taint_sources = Vec::new();
        // This is a loop because our tainted value can appear more than once
        for (idx, &used) in state.ssa_method.uses(ins_id).iter().enumerate() {
            // As stated above, drop idx 0 if it's non-static, otherwise drop all parameters that
            // are not our tainted value.
            if used != tv.value || (idx == 0 && has_receiver) {
                continue;
            }
            taint_sources.push(TaintSource::Param {
                register: idx as u16,
            });
        }

        if taint_sources.is_empty() {
            return propagated;
        }

        propagated | self.descend_into(state, tv.source, method, &taint_sources, path, call)
    }

    fn descend_into<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        source: &'b TaintSource,
        method: &MethodSpec,
        sources: &[TaintSource],
        path: Option<PathId>,
        call: &mut CallState<'_>,
    ) -> bool {
        // Don't go into abstract/native methods and don't go into methods we've already seen in
        // this walk to prevent infinite recursion.
        if !method.has_body() || call.stack.contains(&method.id) {
            return false;
        }

        if call.stack.len() >= self.max_depth {
            log::debug!(
                "hit the call depth limit at {}->{}({}), treating the call as terminal",
                method.class,
                method.name,
                method.signature
            );
            return false;
        }

        log::trace!(
            "Recursing into {}->{}({}) in {}",
            method.class,
            method.name,
            method.signature,
            method.source,
        );

        call.stack.push(method.id);
        let res = self.run_inner(method, sources, call);
        call.stack.pop();

        let Ok(res) = res else {
            log::debug!(
                "failed to analyze {}->{}({}), creating terminal call",
                method.class,
                method.name,
                method.signature,
            );
            return false;
        };

        // Nothing came back, which happens when the tainted argument goes nowhere. The call is
        // still where this value ended up, so record it rather than losing the whole path.
        if res.is_empty() {
            return false;
        }

        // Materialize the targets path to append it to the current one
        let prefix = state.paths.materialize(path);

        for new_paths in res.0.into_values() {
            for new_path in new_paths {
                let mut full = prefix.clone();
                full.extend(new_path);
                state.path.add_path(source.clone(), full);
            }
        }

        true
    }

    /// Every method matching the call target, or `None` if we don't intend to descend into it or
    /// the lookup failed.
    fn resolve_callee(&self, target: &MethodRef<'_>) -> Option<Vec<MethodSpec>> {
        // We're not trying to analyze Android or Java internals, this could probably be expanded a
        // bit
        const TERMINAL_NAMESPACES: &'static [&'static str] = &[
            "Ljava/",
            "Landroid/os/",
            "Landroid/content/res/",
            "Landroid/support/",
        ];

        if TERMINAL_NAMESPACES
            .iter()
            .any(|it| target.class.starts_with(it))
        {
            log::debug!("Skipping terminal class: {}", target.class);
            return None;
        }

        let class = ClassName::from(target.class);
        let search = MethodSearch::new(
            MethodSearchParams::ByFullSpec {
                class: &class,
                name: target.name,
                signature: target.args,
            },
            // Search over all sources, not just ours
            None,
            None,
        );

        match self.db.get_methods(&search) {
            Ok(v) => Some(v),
            Err(e) => {
                log::error!(
                    "failed to find the method {}->{}({}): {}",
                    target.class,
                    target.name,
                    target.args,
                    e
                );
                None
            }
        }
    }

    /// Prefer an implementation from the same source as the caller, then the framework.
    fn pick_callee<'m>(
        &self,
        methods: &'m [MethodSpec],
        state: &RunState<'_, '_>,
        target: &MethodRef<'_>,
    ) -> Option<&'m MethodSpec> {
        if methods.len() == 1 {
            return Some(&methods[0]);
        }

        let source = &state.method.source;
        let found = methods
            .iter()
            .find(|it| it.source.as_str() == source)
            .or_else(|| {
                methods
                    .iter()
                    .find(|it| it.source.as_str() == FRAMEWORK_SOURCE)
            });

        if found.is_none() {
            log::debug!(
                "the target method {}->{}({}) wasn't in {source} or the framework, \
                 treating the call as external",
                target.class,
                target.name,
                target.args
            );
        }

        found
    }
}

/// Index every field read in the method by the field it reads.
fn index_field_reads<'m>(ssa_method: &SsaMethod<'m>) -> FieldReadIndex<'m> {
    let mut index = FieldReadIndex::default();

    for block in ssa_method.reverse_post_order() {
        for ins in ssa_method.block_instructions(block) {
            let inv = ssa_method.instruction(ins);
            if !inv.gets_field() {
                continue;
            }
            let Some(target) = field_ref(inv) else {
                log::warn!("unexpected format for instruction: {}", inv.instruction());
                continue;
            };
            index
                .entry((target.class, target.name))
                .or_default()
                .extend(ssa_method.defs(ins).iter().copied());
        }
    }

    index
}

fn field_ref<'a, 'b>(inv: &'a Invocation<'b>) -> Option<&'a FieldRef<'b>> {
    let field_ref = match inv.args() {
        InvArgs::OneRegField(_, field_ref) => field_ref,
        InvArgs::TwoRegField(_, _, field_ref) => field_ref,
        _ => return None,
    };
    Some(field_ref)
}

fn call_target<'a, 'b>(inv: &'a Invocation<'b>) -> Option<&'a MethodRef<'b>> {
    match inv.args() {
        InvArgs::VarRegMethod(_, mref) => Some(mref),
        InvArgs::Polymorphic(_, mref, _, _) => Some(mref),
        _ => None,
    }
}

/// Whether a tainted argument to this call also taints the object it was called on
///
/// Everything listed here is a framework method whose body holds nothing worth walking, so a match
/// is treated as terminal too. This allows tainting receivers without causing a complexity
/// explosion if we were to taint them whenever.
fn mutates_receiver(class: &str, name: &str) -> bool {
    match class {
        // Every builder style class: the value is accumulated into the receiver
        "Ljava/lang/StringBuilder;" | "Ljava/lang/StringBuffer;" => {
            matches!(name, "append" | "insert" | "replace")
        }

        // Bundles and Intents are the common carriers for IPC data
        "Landroid/os/Bundle;" | "Landroid/os/BaseBundle;" | "Landroid/os/PersistableBundle;" => {
            name.starts_with("put")
        }
        "Landroid/content/Intent;" => {
            name.starts_with("put")
                || name.starts_with("add")
                || name.starts_with("set")
                || name.starts_with("remove")
        }

        // Collections hold whatever is put into them
        "Ljava/util/List;"
        | "Ljava/util/ArrayList;"
        | "Ljava/util/LinkedList;"
        | "Ljava/util/Collection;"
        | "Ljava/util/Set;"
        | "Ljava/util/HashSet;" => {
            matches!(name, "add" | "addAll" | "set" | "offer" | "push")
        }
        "Ljava/util/Map;" | "Ljava/util/HashMap;" | "Ljava/util/LinkedHashMap;" => {
            matches!(name, "put" | "putAll" | "putIfAbsent" | "merge")
        }

        // ContentValues sink into a lot of interesting places, SQL in particular
        "Landroid/content/ContentValues;" => matches!(name, "put"),

        // Buffers and streams accumulate their input
        "Ljava/nio/ByteBuffer;" | "Ljava/nio/CharBuffer;" => name.starts_with("put"),
        "Ljava/io/ByteArrayOutputStream;"
        | "Ljava/io/OutputStream;"
        | "Ljava/io/DataOutputStream;"
        | "Ljava/io/Writer;"
        | "Ljava/io/StringWriter;"
        | "Ljava/io/PrintWriter;" => {
            name.starts_with("write") || matches!(name, "print" | "println")
        }

        // Uri.Builder accumulates path and query components
        "Landroid/net/Uri$Builder;" => {
            name.starts_with("append")
                || matches!(name, "path" | "query" | "fragment" | "encodedPath")
        }

        _ => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_taint_source_round_trips_through_its_display() {
        for text in [
            "p0",
            "p12",
            "android.content.Intent->getStringExtra(Ljava/lang/String;)",
            "*->getIntent()",
            "*->getIntent()Landroid/content/Intent;",
            "android.content.Intent->getExtras()",
            "android.app.Activity->getIntent()Landroid/content/Intent;",
            "com.example.Thing->path",
        ] {
            let parsed: TaintSource = text.parse().expect(text);
            assert_eq!(parsed.to_string(), text, "round trip of {text}");
        }
    }

    /// A return type narrows the match; leaving it off matches any
    #[test]
    fn test_method_call_source_return_type_is_optional() {
        let with_ret: TaintSource = "*->getIntent()Landroid/content/Intent;".parse().unwrap();
        let TaintSource::MethodCall { ret, args, .. } = &with_ret else {
            panic!("expected a MethodCall, got {with_ret:?}");
        };
        assert_eq!(args, "");
        assert_eq!(ret.as_deref(), Some("Landroid/content/Intent;"));

        let without: TaintSource = "*->getIntent()".parse().unwrap();
        let TaintSource::MethodCall { ret: None, .. } = &without else {
            panic!("a missing return type should parse to None, got {without:?}");
        };
        assert_ne!(with_ret, without);

        // Arguments and return type are not confused for one another
        let both: TaintSource = "*->foo(Ljava/lang/String;I)[B".parse().unwrap();
        let TaintSource::MethodCall { args, ret, .. } = &both else {
            panic!("expected a MethodCall, got {both:?}");
        };
        assert_eq!(args, "Ljava/lang/String;I");
        assert_eq!(ret.as_deref(), Some("[B"));
    }

    #[test]
    fn test_taint_source_parse_rejects_junk() {
        for text in ["", "nonsense", "->foo", "foo->", "a->b(c"] {
            assert!(
                text.parse::<TaintSource>().is_err(),
                "{text:?} should not parse"
            );
        }
    }
}
