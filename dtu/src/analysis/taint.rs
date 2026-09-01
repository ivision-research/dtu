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

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display};
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::bail;
use crossbeam::channel::bounded;
use crossbeam::select;
use diesel::prelude::*;
use itertools::Itertools;
use lru::LruCache;
use rayon::ThreadPoolBuilder;
use smalisa::cfg::InstructionId;
use smalisa::instructions::{
    InvArgs, Invocation, INS_INVOKE_CUSTOM, INS_INVOKE_CUSTOM_RANGE, INS_INVOKE_STATIC,
    INS_INVOKE_STATIC_RANGE,
};
use smalisa::ssa::{PhiId, SsaMethod, Value, ValueId, ValueUser};
use smalisa::{AccessFlag, FieldRef, MethodRef, Primitive, Register, SmaliClassName, Type};

use crate::analysis::db::taint::db::{GraphTaintAnalysisDb, UnresolvedOrigin};
use crate::analysis::db::taint::models::MethodStatus;
use crate::analysis::db::taint::schema::{analyzed_methods, run_info};
use crate::analysis::db::taint::writer::{PlannedMethod, RunMeta, TaintWriter};
use crate::analysis::utils::get_ssa_method;
use crate::analysis::CacheStats;
use crate::db::graph::{
    models::{FieldAccessOp, FieldSearch, FieldSearchParams, MethodCallPath, MethodId},
    GraphDatabase, MethodSearch, MethodSearchParams, FRAMEWORK_SOURCE,
};
use crate::db::query;
use crate::tasks::{cancelable_recv, cancelable_send, TaskCancelCheck};
use crate::utils::{opt_deny, OptDenylist};
use crate::Context;
use crate::{analysis::SsaClassLoader, db::graph::MethodSpec, utils::ClassName};

// There is still some stuff to do in here to make this better and more efficient. If you try to run
// taint analysis on very complicated method it will probably blow up in your face. A few things
// that could probably help off the top of my head:

#[derive(PartialEq, Eq, Hash, Ord, PartialOrd, Debug, Clone, serde::Serialize)]
pub enum TaintSource {
    /// An input parameter specified as the smali register number. For example p1 would be
    /// `Param { register: 1 }`
    Param { register: u16 },
    /// The result of a method call inside the function
    ///
    /// The class and return type can be None to match any
    MethodCall {
        class: Option<ClassName>,
        method: String,
        args: String,
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
/// Seeding exists to find methods to analyze via the call graph. It creates entries by searching
/// for calls to a given method from the provided entrypoints. The reason to enable seeding can be
/// seen with a simple example:
///
/// Consider you want to find all calls to `Runtime.exec(String)` with a tainted parameter, and you
/// target will likely only be able to get a tainted value via `Intent.getStringExtra`. _Without_
/// seeding, you would only ever pick up direct calls to `Intent.getStringExtra` from your source
/// method. This is _very_ limited and also very likely not what you want. If a helper method such
/// as `Helpers.getArgument(Intent)` exists and _it_ calls `Intent.getStringExtra`, you would miss
/// it unless you had somehow marked that `Intent` as tainted in some other way (this is
/// straightforward with param taint, seeding exists for the case where param taint can't be used).
/// With seeding enabled, the graph would be queried to ask "are there any methods from my entry
/// method that reach Intent.getStringExtra?" and if so, those methods will be added to the list of
/// methods to be analyzed.
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct TaintSeedOptions {
    /// Restrict target lookups to these sources, empty for any
    pub sources: Vec<String>,
    /// Never seed methods in these classes, even when the graph reaches them
    pub deny_classes: OptDenylist<ClassName>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TaintAnalyzerOptions {
    pub num_threads: usize,
    /// If this is `Some`, seeding will be performed. See [TaintSeedOptions] for what seeding is and
    /// why it may be something you want.
    pub seed: Option<TaintSeedOptions>,
    depth: usize,
}

impl Default for TaintAnalyzerOptions {
    fn default() -> Self {
        let num_threads = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(1))
            .unwrap_or(4);
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
#[derive(Debug, Clone)]
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
    /// The tainted value is stored in a field
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

#[derive(Debug, Clone)]
pub struct TaintSink {
    /// The method the sink occurred in
    pub location: MethodId,
    pub kind: TaintSinkKind,
}

/// One route a tainted value took
///
/// The first entry is the source and the last is where it was last resolved to. Note that the list
/// in sinks may be incomplete for multiple reasons and [Self::incomplete] will let you know if it
/// is. The two main reasons a route might be incomplete are hitting the max call depth or entering
/// a method that is marked as terminal in this module. Various methods are marked as terminals to
/// keep analysis moving along instead of analyzing deeper Android internals.
#[derive(Debug, Clone)]
pub struct TaintRoute {
    pub incomplete: bool,
    pub sinks: Vec<TaintSink>,
}

impl TaintRoute {
    fn new(incomplete: bool, sinks: Vec<TaintSink>) -> Self {
        Self { incomplete, sinks }
    }

    fn extend(&mut self, other: Self) {
        self.incomplete |= other.incomplete;
        self.sinks.extend(other.sinks);
    }
}

impl Deref for TaintRoute {
    type Target = Vec<TaintSink>;
    fn deref(&self) -> &Self::Target {
        &self.sinks
    }
}

impl DerefMut for TaintRoute {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.sinks
    }
}

/// The taint that spread from one [TaintSource]
#[derive(Clone)]
pub struct TaintSourceAndRoute {
    pub source: TaintSource,
    pub paths: Vec<TaintRoute>,
}

/// The taint analysis results for a single method
#[derive(Clone)]
pub struct MethodTaint {
    pub method: MethodId,
    pub origin: UnresolvedOrigin,
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

struct CallState {
    stack: Vec<MethodId>,
}

/// What we decided to do upon seeing a call instruction
#[derive(Clone, Copy)]
enum PropagationDecision {
    /// The call was terminal, we didn't descend into the callee
    Terminal,
    /// The call wasn't terminal, but we didn't descend into the callee because we hit the depth
    /// limit
    Truncated,
    /// The call wasn't terminal and we descended into the callee or we added the return value as
    /// tainted
    Propagated,
}

impl PropagationDecision {
    fn from_bool(propagated: bool) -> Self {
        if propagated {
            Self::Propagated
        } else {
            Self::Terminal
        }
    }

    /// Combine a boolean propagation decision with an actual [PropagationDecision]
    ///
    /// If the passed value is [Self::Terminal], take whatever the propagated boolean says,
    /// otherwise take whatever the [PropagationDecision] says. This is a bit of an ugly function,
    /// but it's only used in one place and it kinda makese sense there. The idea is basically
    /// that the propagated flag overrides terminal
    fn combine(propagated: bool, other: Self) -> Self {
        match other {
            Self::Terminal if propagated => Self::Propagated,
            _ => other,
        }
    }
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
    origins: HashMap<MethodId, UnresolvedOrigin>,
}

impl WorkPlan {
    fn sources_for(&self, method: &MethodSpec) -> &[TaintSource] {
        self.seeds.get(&method.id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn origin_for(&self, method: &MethodSpec) -> UnresolvedOrigin {
        self.origins
            .get(&method.id)
            .cloned()
            .unwrap_or(UnresolvedOrigin::Direct)
    }

    /// Add a method the call graph reached, merging into what is already planned for it
    ///
    /// A method that was asked for directly keeps that origin: it is already being analyzed with
    /// its own seeds and the chains that reach it say nothing extra.
    fn add_indirect(&mut self, method: MethodSpec, source: TaintSource, chain: MethodCallPath) {
        let ids = chain.path.iter().map(|it| it.id).collect::<Vec<MethodId>>();

        match self.origins.entry(method.id) {
            Entry::Occupied(mut existing) => {
                if let UnresolvedOrigin::CallGraph { chains } = existing.get_mut() {
                    chains.push(ids);
                } else {
                    // UnresolvedOrigin::Direct means this method is already being analyzed directly so
                    // nothing to add here.
                    return;
                }
            }
            Entry::Vacant(slot) => {
                // We found a completely new method to analyze as an entrypoint
                slot.insert(UnresolvedOrigin::CallGraph { chains: vec![ids] });
                self.methods.push(method.clone());
            }
        }

        // May have discovered more sources? I think?
        let sources = self.seeds.entry(method.id).or_default();
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
}

/// Entrypoint for performing taint analysis
pub struct TaintAnalyzer<'a> {
    ctx: &'a dyn Context,
    cancel: TaskCancelCheck,
    opts: TaintAnalyzerOptions,
    loader: Arc<SsaClassLoader>,
    seeds: &'a dyn TaintSeeds,
}

struct TaintAnalyzerWorker<'a> {
    ctx: &'a dyn Context,
    max_depth: usize,
    cancel: &'a TaskCancelCheck,
    loader: Arc<SsaClassLoader>,
}

impl<'a> TaintAnalyzer<'a> {
    pub fn new(
        ctx: &'a dyn Context,
        cancel: TaskCancelCheck,
        opts: TaintAnalyzerOptions,
        seeds: &'a dyn TaintSeeds,
        loader: Arc<SsaClassLoader>,
    ) -> Self {
        Self {
            ctx,
            cancel,
            opts,
            seeds,
            loader,
        }
    }

    /// Work out everything that will be analyzed, before any of it starts
    fn plan(&self, methods: Vec<MethodSpec>, db: &GraphTaintAnalysisDb) -> WorkPlan {
        let mut plan = WorkPlan::default();

        // Don't include methods that we've already worked on in the plan.
        let mut done_methods = db
            .query(|c| {
                query!(analyzed_methods::table
                    .select(analyzed_methods::method)
                    .filter(analyzed_methods::status.eq(MethodStatus::Done)))
                .get_results::<MethodId>(c)
            })
            .unwrap_or(vec![])
            .iter()
            .copied()
            .collect::<HashSet<_>>();

        // First seed the provided methods directly with their taint sources
        for method in methods {
            // Ignore methods we've already done _and_ deduplicate the input methods
            if !done_methods.insert(method.id) {
                continue;
            }
            plan.seeds
                .insert(method.id, self.seeds.sources_for(&method).to_vec());
            // These are all direct calls
            plan.origins.insert(method.id, UnresolvedOrigin::Direct);
            plan.methods.push(method);
        }

        // The existence of `seed` means "yes do seeding via the graph"
        let Some(opts) = self.opts.seed.as_ref() else {
            return plan;
        };

        // Get all of our method database IDs for some queries
        let entries = plan.methods.iter().map(|it| it.id).collect::<Vec<_>>();

        // Collect the seeds into a hash set here to both make them unique and prevent a borrow
        // error over in `plan.add_indirect`.
        let sources = plan
            .seeds
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>();

        for source in sources {
            for chain in self.reaching_chains(db.graph(), &source, &entries, opts) {
                // The last element of the chain is the direct call into the method we're interested
                // in
                let Some(method) = chain.path.last().cloned() else {
                    continue;
                };

                // Don't do work we've already done.
                if done_methods.contains(&method.id) {
                    continue;
                }

                // This may seem a bit silly, we just did a search for a call to that method, but
                // remember methods can be searched for with no class set. See the usage for
                // activities in the CLI for why this is useful.
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
    /// This is the core action for seeding, see [TaintSeedOptions] for what seeding means
    fn reaching_chains(
        &self,
        gdb: &dyn GraphDatabase,
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
                // Are there any paths to the taint source from the entrypoint?
                gdb.find_callers_from(&search, entries)
            } else if let Some(search) = source.field_search(scope) {
                // Are there any reads of the tainted field deeper in the call graph?
                gdb.find_field_refs_from(&search, FieldAccessOp::Read, entries)
            } else {
                // A Param only means something against the signature it was given for
                continue;
            };
            // If we have results, we note the path. The path will eventually end up in
            // [UnresolvedOrigin::CallGraph], we'll use the last element of the path to seed the
            // analysis though
            match res {
                Ok(chains) => found.extend(chains),
                Err(e) => log::warn!("failed to expand seeds for {source}: {e}"),
            }
        }
        found
    }

    /// Run the analysis
    ///
    /// Analyze `methods`, writing each result into `db` as it completes
    pub fn run(
        &mut self,
        methods: Vec<MethodSpec>,
        db: GraphTaintAnalysisDb,
        meta: &RunMeta,
    ) -> anyhow::Result<GraphTaintAnalysisDb> {
        let is_done = db
            .query(|c| {
                query!(run_info::table.select(run_info::completed))
                    .get_result::<bool>(c)
                    .optional()
            })?
            .is_some_and(|it| it);

        if is_done {
            log::info!("Database already marked as complete");
            return Ok(db);
        }

        let plan = self.plan(methods, &db);
        let planned = plan
            .methods
            .iter()
            .map(|it| PlannedMethod {
                method: it.id,
                origin: plan.origin_for(it),
            })
            .collect::<Vec<PlannedMethod>>();

        let gdb = db.graph();
        let mut writer = TaintWriter::update_run_meta(&db, &meta.as_insert(), &planned)?;

        let nthreads = self.opts.num_threads.max(1);
        log::info!(
            "Running on {} methods with {} threads",
            plan.methods.len(),
            nthreads
        );

        let worker_pool = ThreadPoolBuilder::new()
            .num_threads(nthreads)
            .build()
            .expect("building worker pool");

        let (work_tx, work_rx) = bounded(2 * worker_pool.current_num_threads());
        let (fail_tx, fail_rx) =
            bounded::<(MethodSpec, String)>(2 * worker_pool.current_num_threads());
        let (result_tx, result_rx) =
            bounded::<(MethodSpec, MethodPaths)>(2 * worker_pool.current_num_threads());

        thread::scope(|s| {
            // TODO failures on `record*` functions..?
            s.spawn(|| {
                let mut it = plan.methods.iter().cloned();

                'outer: while let Some(method) = it.next() {
                    if self.cancel.was_cancelled() {
                        break;
                    }
                    loop {
                        select! {
                            recv(fail_rx) -> res => {
                                if let Ok((method, reason)) = res {
                                    if let Err(e) = record_failure(&mut writer, method, reason) {
                                        log::error!("failed to record .. well a failure: {e})");
                                    }
                                }
                            },
                            recv(result_rx) -> res => {
                                if let Ok((method, path)) = res {
                                    if let Err(e) = record(&mut writer, &plan, method, path) {
                                        log::error!("failed to record a result: {e}");
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
                            if let Ok((method, reason)) = res {
                                if let Err(e) = record_failure(&mut writer, method, reason) {
                                    log::error!("failed to record .. well a failure: {e})");
                                }
                            }
                        },
                        recv(result_rx) -> res => {
                            if let Ok((method, path)) = res {
                                if let Err(e) = record(&mut writer, &plan, method, path) {
                                    log::error!("failed to record a result: {e}");
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

            worker_pool.broadcast(|_| {
                let mut resolver = MethodResolver::new(gdb);
                let analyzer = TaintAnalyzerWorker {
                    ctx: self.ctx,
                    max_depth: self.opts.depth,
                    cancel: &self.cancel,
                    loader: Arc::clone(&self.loader),
                };

                loop {
                    let Ok(Some(method)) = cancelable_recv(analyzer.cancel, &work_rx) else {
                        break;
                    };
                    let sources = plan.sources_for(&method);
                    match analyzer.run(&method, sources, &mut resolver) {
                        Err(e) => {
                            log::debug!("failed to analyze method {method}: {e}");
                            let Ok(true) =
                                cancelable_send(analyzer.cancel, (method, e.to_string()), &fail_tx)
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
            });

            drop(fail_tx);
            drop(result_tx);
        });

        writer.rebuild_fts();

        if self.cancel.was_cancelled() {
            return Ok(db);
        }

        let nincomplete = writer.db.query(|c| {
            analyzed_methods::table
                .count()
                .filter(analyzed_methods::status.ne(MethodStatus::Done))
                .get_result::<i64>(c)
        })?;

        if nincomplete == 0 {
            writer.finish()?;
        }

        Ok(db)
    }
}

fn record_failure(
    writer: &mut TaintWriter,
    method: MethodSpec,
    reason: String,
) -> anyhow::Result<()> {
    writer.mark_failed(method.id, &reason)?;
    Ok(())
}

fn record(
    writer: &mut TaintWriter,
    plan: &WorkPlan,
    method: MethodSpec,
    path: MethodPaths,
) -> anyhow::Result<()> {
    let origin = plan.origin_for(&method);
    let taint = path.into_taint(method.id, origin);
    writer.write_method(&taint)?;
    writer.mark_done(method.id)?;
    Ok(())
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

    fn into_taint(self, method: MethodId, origin: UnresolvedOrigin) -> MethodTaint {
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

impl PathId {
    const TRUNCATE_BIT: usize = 31;
    fn index(self) -> usize {
        (self.0 & (!(1 << Self::TRUNCATE_BIT))) as usize
    }

    fn truncate(&mut self) {
        self.0 |= 1 << Self::TRUNCATE_BIT
    }

    fn is_truncated(self) -> bool {
        self.0 & (1 << Self::TRUNCATE_BIT) != 0
    }
}

struct PathNode {
    sink: TaintSink,
    parent: Option<PathId>,
}

/// The sinks a tainted value has passed through, stored as a tree rather than a vec per path.
///
/// A value with several users forks the path, and every fork shares the prefix that led to it.
/// Holding the prefix once means extending is a single push and forking is a copy of a [PathId],
/// instead of cloning the whole history at every branch. [PathBuilder::materialize] walks back to the
/// root, and only runs when a finished path is recorded.
#[derive(Default)]
struct PathBuilder {
    nodes: Vec<PathNode>,
}

impl PathBuilder {
    fn push(&mut self, parent: Option<PathId>, sink: TaintSink) -> PathId {
        let id = PathId(self.nodes.len() as u32);
        self.nodes.push(PathNode { sink, parent });
        id
    }

    /// The path from the source to `from`, in the order it was walked.
    fn materialize(&self, from: Option<PathId>) -> TaintRoute {
        let mut out = Vec::new();
        let mut at = from;
        let incomplete = from.is_some_and(PathId::is_truncated);
        while let Some(id) = at {
            let node = &self.nodes[id.index()];
            out.push(node.sink.clone());
            at = node.parent;
        }
        out.reverse();
        TaintRoute::new(incomplete, out)
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
    paths: PathBuilder,
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
            paths: PathBuilder::default(),
        }
    }

    /// Record a finished path only if the walk isn't continuing past here.
    ///
    /// If taint moved on to another value the longer path will be recorded when it ends, and
    /// recording here as well would report a truncated duplicate.
    fn record_unless(
        &mut self,
        decision: PropagationDecision,
        source: &TaintSource,
        path: Option<PathId>,
    ) {
        // Terminal and Truncated cause us to record the path
        if !matches!(decision, PropagationDecision::Propagated) {
            self.record(
                source,
                path,
                matches!(decision, PropagationDecision::Truncated),
            );
        }
    }

    /// Record a finished path for `source`
    fn record(&mut self, source: &TaintSource, mut path: Option<PathId>, truncated: bool) {
        if truncated {
            if let Some(mpath) = path.as_mut() {
                mpath.truncate();
            }
        }

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

    fn push_phi(&mut self, source: &'b TaintSource, phi: PhiId, path: Option<PathId>) {
        let value = self.ssa_method.phi(phi).value;
        // Phis can't be terminal, so only ever queue them up for more work, never record. This
        // means that some phis can just kinda vanish into the ether. That's ok, terminal phis are
        // not particularly helpful in the output.
        let tv = TaintedValue::new(source, value);
        if self.seen.insert(tv) {
            let path = self.extend(path, TaintSinkKind::Phi);
            self.work.push((tv, Some(path)));
        }
    }

    fn push_array(&mut self, source: &'b TaintSource, ins_id: InstructionId, path: Option<PathId>) {
        // Arrays are not terminal, while it may be interesting that something entered an array, it
        // is more interesting where that array goes. We handle that by marking the array itself as
        // tainted.

        // The array itself is always the first argument
        let Some(val) = self.ssa_method.uses(ins_id).first() else {
            return;
        };
        // Mark the array as tainted if it isn't already tainted
        let tv = TaintedValue::new(source, *val);
        if self.seen.insert(tv) {
            let path = self.extend(path, TaintSinkKind::Array);
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
        self.record(source, Some(path), false);

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
        // As things stand right now, instructions wont't be considered terminal. The reason for
        // that is that anything interesting happens after the instruction and we cover that by
        // tainting the outputs of the instruction. After looking at some outputs where the
        // instructions could be terminal, they were basically always just noise.
        let ins = self.ssa_method.instruction(ins_id).instruction();
        let path = self.extend(
            path,
            TaintSinkKind::Instruction {
                opcode: ins.to_string(),
            },
        );

        // We'll consider all outputs from instructions with tainted input to be tained. I'm not
        // sure this is the best way to handle this but that's ok.
        for &out in self.ssa_method.defs(ins_id) {
            let new_tv = TaintedValue::new(tv.source, out);
            if self.seen.insert(new_tv) {
                self.work.push((new_tv, Some(path)));
            }
        }
    }
}

#[derive(Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Clone)]
#[repr(transparent)]
struct MethodKey(String);

impl MethodKey {
    fn new() -> Self {
        Self(String::with_capacity(256))
    }

    fn set(&mut self, mref: &MethodRef, source: &str) {
        self.0.clear();
        let ty = &mref.return_type;
        let smali_ty = match ty.as_smali_str() {
            Some(v) => v,
            None => Cow::Borrowed(""),
        };

        for part in [mref.class, mref.name, mref.args, &smali_ty, source] {
            self.0.push_str(part);
            // NULL can't appear in valid smali so it's a good sep here, otherwise we could
            // accidentally get weirdness
            self.0.push('\0');
        }
    }
}

struct MethodResolver<'a> {
    cache: LruCache<MethodKey, MethodSpec>,
    missing: HashSet<MethodKey>,
    gdb: &'a dyn GraphDatabase,
    source: Option<String>,
    source_is_framework: bool,
    stats: CacheStats,

    key: MethodKey,
}

struct OldSource {
    source: Option<String>,
    is_framework: bool,
}

impl<'a> MethodResolver<'a> {
    fn new(gdb: &'a dyn GraphDatabase) -> Self {
        let cache = LruCache::new(NonZeroUsize::new(512).unwrap());
        let source_is_framework = false;
        Self {
            source: None,
            gdb,
            cache,
            key: MethodKey::new(),
            missing: HashSet::new(),
            source_is_framework,
            stats: CacheStats::new(),
        }
    }

    fn set_source(&mut self, source: &str) -> OldSource {
        let old = self.source.replace(String::from(source));
        let is_framework = self.source_is_framework;
        self.source_is_framework = source == FRAMEWORK_SOURCE;
        OldSource {
            source: old,
            is_framework,
        }
    }

    fn restore_source(&mut self, old: OldSource) {
        self.source = old.source;
        self.source_is_framework = old.is_framework;
    }

    fn resolve_method(&mut self, target: &MethodRef) -> Option<MethodSpec> {
        // This list is used to stop descent into classes that match. There is no reason to delve
        // deep into Android or Java internals, and some well know libraries can be included here as
        // well. Note that this is a bit of a balance: we're analyzing entire devices so it is
        // definitely possible a common namespace got cluttered with vendor code that shouldn't be
        // there but is.
        const TERMINAL_NAMESPACES: &'static [&'static str] = &[
            "Ljava/",
            "Ljavax/",
            "Lsun/",
            "Llibcore/",
            "Lorg/json/",
            "Lorg/xml/",
            "Lorg/bouncycastle/",
            "Lkotlin/",
            "Lkotlinx/",
            "Lcom/google/protobuf/",
            "Lcom/google/common/",
            "Lcom/google/gson/",
            // These Android specific ones are the most likely to hide vendor code that we wanted to
            // see, but I don't see that happen often enough to matter and it's definitely worth not
            // descending into almost all of  these.
            "Ldalvik/",
            "Landroidx/",
            "Landroid/os/",
            "Landroid/content/res/",
            "Landroid/support/",
            // This one was a can of worms if something ever hit it
            "Lcom/android/internal/pm/",
            // Not a namespace, just don't descend into log..
            "Landroid/util/Log;",
        ];

        if TERMINAL_NAMESPACES
            .iter()
            .any(|it| target.class.starts_with(it))
        {
            log::debug!("Skipping terminal class: {}", target.class);
            return None;
        }

        let source = self
            .source
            .as_ref()
            .map(String::as_str)
            .unwrap_or(FRAMEWORK_SOURCE);

        self.stats.lookup_attempt();

        self.key.set(target, source);

        if let Some(cached) = self.cache.get(&self.key) {
            self.stats.lru_hit();
            return Some(cached.clone());
        };

        if self.missing.contains(&self.key) {
            return None;
        }

        let class = ClassName::from(target.class);

        // This only actually returns None when the type is undefined which should never happen
        let return_type = match target.return_type.as_smali_str() {
            Some(v) => v,
            None => Cow::Borrowed("V"),
        };

        match self.gdb.get_method_source_or_framework(
            &class,
            target.name,
            target.args,
            &return_type,
            source,
        ) {
            Ok(Some(v)) => {
                self.cache.put(self.key.clone(), v.clone());
                Some(v)
            }
            Ok(None) => {
                self.missing.insert(self.key.clone());
                log::warn!(
                    "method {}->{}({}): not in database",
                    target.class,
                    target.name,
                    target.args,
                );
                None
            }
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
}

impl<'a> TaintAnalyzerWorker<'a> {
    fn run(
        &self,
        method: &MethodSpec,
        sources: &[TaintSource],
        resolver: &mut MethodResolver,
    ) -> anyhow::Result<MethodPaths> {
        let mut call = CallState { stack: Vec::new() };
        resolver.set_source(&method.source);
        self.run_inner(method, sources, resolver, &mut call)
    }

    fn run_inner_no_really_this_time(
        &self,
        method: &MethodSpec,
        sources: &[TaintSource],
        resolver: &mut MethodResolver,
        call: &mut CallState,
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
            if self.cancel.was_cancelled() {
                break;
            }
            let users = ssa_method.users(tv.value);

            // Nothing consumes this value, so the path ends here
            if users.is_empty() {
                state.record(tv.source, tv_path, false);
                continue;
            }

            self.on_users(&mut state, users, tv, tv_path, resolver, call);
        }

        Ok(state.path)
    }

    fn run_inner(
        &self,
        method: &MethodSpec,
        sources: &[TaintSource],
        resolver: &mut MethodResolver,
        call: &mut CallState,
    ) -> anyhow::Result<MethodPaths> {
        log::debug!("Starting {} on method {method}", sources.iter().join(", "));
        let res = self.run_inner_no_really_this_time(method, sources, resolver, call);
        log::debug!("Done with {} on method {method}", sources.iter().join(", "));
        res
    }

    /// Follow a tainted value into every one of its users.
    fn on_users<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        users: &'_ [ValueUser],
        tv: TaintedValue<'b>,
        path: Option<PathId>,
        resolver: &mut MethodResolver,
        call: &mut CallState,
    ) {
        for &user in users {
            match user {
                ValueUser::Phi(phi_id) => {
                    state.push_phi(tv.source, phi_id, path);
                }
                ValueUser::Instruction(ins_id) => {
                    let inv = state.ssa_method.instruction(ins_id);
                    if inv.is_call() {
                        self.on_call(state, ins_id, inv, tv, path, resolver, call);
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
        resolver: &mut MethodResolver,
        call: &mut CallState,
    ) {
        let Some(target) = call_target(inv) else {
            log::warn!("instruction {} returned no call target", inv.instruction());
            state.record(tv.source, path, false);
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
        let method = resolver.resolve_method(target);
        let callee = method.as_ref();

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

        let decision = self.propagate(
            state,
            ins_id,
            tv,
            Some(call_path),
            call_data,
            callee,
            resolver,
            call,
        );
        state.record_unless(decision, tv.source, Some(call_path));
    }

    fn propagate<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        ins_id: InstructionId,
        tv: TaintedValue<'b>,
        path: Option<PathId>,
        call_data: CallData,
        callee: Option<&MethodSpec>,
        resolver: &mut MethodResolver,
        call: &mut CallState,
    ) -> PropagationDecision {
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
            return PropagationDecision::from_bool(propagated);
        }

        let Some(method) = callee else {
            return PropagationDecision::from_bool(propagated);
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
            return PropagationDecision::from_bool(propagated);
        }

        PropagationDecision::combine(
            propagated,
            self.descend_into(
                state,
                tv.source,
                method,
                &taint_sources,
                path,
                resolver,
                call,
            ),
        )
    }

    fn descend_into<'b>(
        &self,
        state: &mut RunState<'b, '_>,
        source: &'b TaintSource,
        method: &MethodSpec,
        sources: &[TaintSource],
        path: Option<PathId>,
        resolver: &mut MethodResolver,
        call: &mut CallState,
    ) -> PropagationDecision {
        // Don't go into abstract/native methods and don't go into methods we've already seen in
        // this walk to prevent infinite recursion.
        if !method.has_body() || call.stack.contains(&method.id) {
            return PropagationDecision::Terminal;
        }

        if call.stack.len() >= self.max_depth {
            log::debug!(
                "hit the call depth limit at {}->{}({}), treating the call as terminal",
                method.class,
                method.name,
                method.signature
            );
            return PropagationDecision::Truncated;
        }

        log::trace!(
            "Recursing into {}->{}({}) in {}",
            method.class,
            method.name,
            method.signature,
            method.source,
        );

        call.stack.push(method.id);
        let old = resolver.set_source(&method.source);
        let res = self.run_inner(method, sources, resolver, call);
        resolver.restore_source(old);
        call.stack.pop();

        let Ok(res) = res else {
            log::debug!(
                "failed to analyze {}->{}({}), creating terminal call",
                method.class,
                method.name,
                method.signature,
            );
            return PropagationDecision::Terminal;
        };

        // Nothing came back, which happens when the tainted argument goes nowhere. The call is
        // still where this value ended up, so record it rather than losing the whole path.
        if res.is_empty() {
            return PropagationDecision::Terminal;
        }

        // Materialize our own path so we can extend it with the resulting paths
        let prefix = state.paths.materialize(path);

        for new_paths in res.0.into_values() {
            for new_path in new_paths {
                // Extend our current path with one of the result's paths
                let mut full = prefix.clone();
                full.extend(new_path);
                // Add it as a path
                state.path.add_path(source.clone(), full);
            }
        }

        PropagationDecision::Propagated
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

        // JSON built from tainted data, note that this could be keys instead of values which are
        // less interesting but we have no way to know
        "Lorg/json/JSONObject;" | "Lorg/json/JSONArray;" => name == "put",
        "Lcom/google/gson/JsonObject;" | "com/google/gson/JsonArray;" => name.starts_with("add"),

        // Fluent, and the receiver holding taint is what makes commit/apply the sink
        "Landroid/content/SharedPreferences$Editor;" => name.starts_with("put"),

        // Android's own containers
        "Landroid/util/SparseArray;" | "Landroid/util/ArrayMap;" | "Landroid/util/ArraySet;" => {
            matches!(name, "put" | "add" | "append" | "putAll" | "addAll")
        }

        "Landroid/os/Message;" => matches!(name, "setData" | "copyFrom"),

        // Collections hold whatever is put into them
        "Ljava/util/Map;"
        | "Ljava/util/HashMap;"
        | "Ljava/util/LinkedHashMap;"
        | "Ljava/util/TreeMap;"
        | "Ljava/util/concurrent/ConcurrentHashMap;" => {
            matches!(name, "put" | "putAll" | "putIfAbsent" | "merge")
        }

        "Ljava/util/List;"
        | "Ljava/util/ArrayList;"
        | "Ljava/util/LinkedList;"
        | "Ljava/util/Collection;"
        | "Ljava/util/Set;"
        | "Ljava/util/HashSet;"
        | "Ljava/util/LinkedHashSet;"
        | "Ljava/util/ArrayDeque;" => {
            matches!(name, "add" | "addAll" | "set" | "offer" | "push")
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
