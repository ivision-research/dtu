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
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::bail;
use crossbeam::channel::{bounded, Sender};
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
use smalisa::{
    AccessFlag, FieldRef, MethodRef, Primitive, Register, RegisterNumber, SmaliClassName, Type,
};

use super::db::UnresolvedOrigin;
use super::models::MethodStatus;
use super::schema::{analyzed_methods, run_info};
use super::writer::{DatabaseAction, PlannedMethod, RawSink, RunMeta, RunWriter};
use super::CacheStats;
use crate::analysis::taint::db::{TaintAnalysisDb, TaintAnalysisDbWriter};
use crate::analysis::taint::id_factory::{new_id_factory, IdFactory};
use crate::analysis::taint::models::{AnalyzedMethodId, NodeId, SubgraphId, TaintSourceId};
use crate::analysis::taint::schema::{graphs, nodes, taint_sources};
use crate::analysis::utils::get_ssa_method;
use crate::db::graph::models::FieldId;
use crate::db::graph::{
    models::{FieldAccessOp, FieldSearch, FieldSearchParams, MethodId},
    GraphDatabase, MethodSearch, MethodSearchParams,
};
use crate::db::{self, query};
use crate::tasks::{cancelable_recv, cancelable_send, TaskCancelCheck};
use crate::utils::{opt_deny, OptDenylist};
use crate::Context;
use crate::{analysis::SsaClassLoader, db::graph::MethodSpec, utils::ClassName};

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

impl TaintSource {
    fn is_param(&self) -> bool {
        matches!(self, Self::Param { .. })
    }

    #[allow(dead_code)]
    fn is_method_call(&self) -> bool {
        matches!(self, Self::MethodCall { .. })
    }

    #[allow(dead_code)]
    fn is_field(&self) -> bool {
        matches!(self, Self::Field { .. })
    }
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

#[derive(Debug, Clone)]
enum ResolvedMethod {
    /// A terminal method, don't propagate into these
    Terminal(MethodSpec),
    /// A method that should be inspected and propagated into: non-terminal
    Inspectable(MethodSpec),
}

impl ResolvedMethod {
    fn get_id(&self) -> MethodId {
        match self {
            Self::Terminal(v) => v.id,
            Self::Inspectable(v) => v.id,
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
    Field { field: FieldId },
    /// The tainted value was passed to a method the graph database knows about. Resolve the id to
    /// get the class and signature back.
    MethodCall { method: MethodId },

    /// The tainted value was passed to a field that was not in the graph database
    ExternalField { class: ClassName, name: String },

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

pub(super) struct IdFactories {
    graph: IdFactory<SubgraphId>,
    nodes: IdFactory<NodeId>,
    sources: IdFactory<TaintSourceId>,
    analyzed_methods: IdFactory<AnalyzedMethodId>,
}

impl IdFactories {
    fn new(db: &TaintAnalysisDb) -> db::Result<Self> {
        let graph = new_id_factory!(db, graphs::table, graphs::id)?;
        let nodes = new_id_factory!(db, nodes::table, nodes::id)?;
        let sources = new_id_factory!(db, taint_sources::table, taint_sources::id)?;
        let analyzed_methods = new_id_factory!(db, analyzed_methods::table, analyzed_methods::id)?;

        Ok(Self {
            nodes,
            graph,
            sources,
            analyzed_methods,
        })
    }

    pub(super) fn new_analyzed_method_id(&self) -> AnalyzedMethodId {
        self.analyzed_methods.next()
    }

    pub(super) fn new_taint_source_id(&self) -> TaintSourceId {
        self.sources.next()
    }

    pub(super) fn new_graph_id(&self) -> SubgraphId {
        self.graph.next()
    }

    pub(super) fn new_node_id(&self) -> NodeId {
        self.nodes.next()
    }
}

#[derive(Debug, Clone, Copy)]
struct TaintedValue {
    /// The taint source for this tainted value
    source: TaintSourceId,
    /// The SSA ValueId
    value: ValueId,
}

// PartialEq, Eq, and Hash all depend only on prev and value

impl PartialEq for TaintedValue {
    fn eq(&self, other: &Self) -> bool {
        self.value.eq(&other.value)
    }
}

impl Eq for TaintedValue {}

impl Hash for TaintedValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl TaintedValue {
    fn new(source: TaintSourceId, value: ValueId) -> Self {
        Self { source, value }
    }

    fn with_value(mut self, value: ValueId) -> Self {
        self.value = value;
        self
    }
}

/// Opaque type for tracking a walk
#[derive(Clone)]
pub struct WalkCookie {
    key: AnalyzedMethod,
    depth: u16,
    truncated: bool,
    new: bool,
}

impl WalkCookie {
    fn new(depth: u16, key: AnalyzedMethod, new: bool) -> WalkCookie {
        Self {
            depth,
            key,
            truncated: false,
            new,
        }
    }

    fn is_new(&self) -> bool {
        self.new
    }

    /// Mark the walk as truncated
    fn truncate(&mut self) {
        self.truncated = true;
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum MethodWalkStatus {
    /// This method was able to be walked to completion
    Complete,
    /// The walk finished and was truncated
    Truncated { depth: u16 },
}

#[derive(PartialEq, Eq, Hash, Clone)]
struct AnalyzedMethod {
    method: MethodId,
    /// The tainted registers associated with the call. This allows differentiating things like M(a,
    /// b, c) where a is tainted from M(a, b, c) where b and c are tainted. These are genuinely
    /// different paths and should betreated as such.
    registers: Vec<RegisterNumber>,
}

type WorkerCacheMap = HashMap<AnalyzedMethod, (NodeId, MethodWalkStatus)>;

struct WorkerCacheState<'a> {
    nodes: &'a IdFactory<NodeId>,
    map: WorkerCacheMap,
}

impl<'a> WorkerCacheState<'a> {
    fn new(nodes: &'a IdFactory<NodeId>) -> Self {
        let map = HashMap::new();
        Self { nodes, map }
    }
}

impl<'a> Deref for WorkerCacheState<'a> {
    type Target = WorkerCacheMap;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<'a> DerefMut for WorkerCacheState<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

/// Global cache shared among all workers
struct WorkerCache<'a> {
    /// A collection of [AnalyzedMethod]s that have already been claimed by another thread. This
    /// significantly limits the amount of work done by not constantly re-analyzing the same paths.
    seen: Mutex<WorkerCacheState<'a>>,
}

impl<'a> WorkerCache<'a> {
    // TODO: This shouldn't take an option, we should always be able to determine what the initial
    // value is from the database
    fn new(nodes: &'a IdFactory<NodeId>) -> Self {
        Self {
            seen: Mutex::new(WorkerCacheState::new(nodes)),
        }
    }

    /// Attempt to claim the key for analysis
    ///
    /// If some other worker is already handling this method with a greater depth budget, this will
    /// return (id, None). Otherwise this will return (id, Some(cookie)).
    fn claim(&self, key: AnalyzedMethod, remaining_depth: usize) -> (NodeId, Option<WalkCookie>) {
        let remaining_depth = remaining_depth as u16;

        let mut state = self.seen.lock().expect("poisoned mutex");
        match state.get(&key) {
            // Something completed the walk already so we're good
            Some((id, MethodWalkStatus::Complete)) => (*id, None),

            Some((id, MethodWalkStatus::Truncated { depth })) => {
                let id = *id;
                if *depth >= remaining_depth {
                    // Some other worker is doing the same method and has more depth budget
                    (id, None)
                } else {
                    let cloned = key.clone();
                    // The new caller has more depth budget than the previous owner so go ahead and
                    // explore further
                    state.insert(
                        key,
                        (
                            id,
                            MethodWalkStatus::Truncated {
                                depth: remaining_depth,
                            },
                        ),
                    );
                    (id, Some(WalkCookie::new(remaining_depth, cloned, false)))
                }
            }

            None => {
                let cloned = key.clone();
                let id = state.nodes.next();

                state.insert(
                    key,
                    (
                        id,
                        MethodWalkStatus::Truncated {
                            depth: remaining_depth,
                        },
                    ),
                );
                (id, Some(WalkCookie::new(remaining_depth, cloned, true)))
            }
        }
    }

    fn record(&self, cookie: WalkCookie) {
        let WalkCookie {
            depth,
            key,
            truncated,
            ..
        } = cookie;
        let mut state = self.seen.lock().expect("poisoned mutex");
        let Some((_, it)) = state.get_mut(&key) else {
            // This really should be unreachable here
            return;
        };

        let new = if truncated {
            MethodWalkStatus::Truncated { depth }
        } else {
            MethodWalkStatus::Complete
        };

        match it {
            // Only update if still truncated
            MethodWalkStatus::Truncated { depth: trunc_depth } if *trunc_depth < depth => *it = new,
            // Don't do anything if it is already complete or some other worker had more depth
            // budget
            _ => {}
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

enum TaintSourceWrapper<'a> {
    Borrowed(&'a TaintSource),
    Owned(TaintSource),
}

impl<'a> Clone for TaintSourceWrapper<'a> {
    fn clone(&self) -> Self {
        match self {
            Self::Borrowed(ts) => Self::Borrowed(*ts),
            Self::Owned(ts) => Self::Owned(ts.clone()),
        }
    }
}

impl<'a> Deref for TaintSourceWrapper<'a> {
    type Target = TaintSource;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(ts) => *ts,
            Self::Owned(ts) => ts,
        }
    }
}

impl<'a> fmt::Display for TaintSourceWrapper<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Borrowed(ts) => (*ts).fmt(f),
            Self::Owned(ts) => ts.fmt(f),
        }
    }
}

impl<'a> PartialEq for TaintSourceWrapper<'a> {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::Borrowed(ts) => match other {
                Self::Borrowed(ots) => (*ts).eq(*ots),
                Self::Owned(ots) => (*ts).eq(ots),
            },
            Self::Owned(ts) => match other {
                Self::Borrowed(ots) => ts.eq(*ots),
                Self::Owned(ots) => ts.eq(ots),
            },
        }
    }
}

impl<'a> Eq for TaintSourceWrapper<'a> {}

impl<'a> From<TaintSource> for TaintSourceWrapper<'a> {
    fn from(value: TaintSource) -> Self {
        Self::Owned(value)
    }
}

impl<'a> From<&'a TaintSource> for TaintSourceWrapper<'a> {
    fn from(value: &'a TaintSource) -> Self {
        Self::Borrowed(value)
    }
}

#[derive(Clone)]
struct IdTaintSource<'a> {
    id: TaintSourceId,
    source: TaintSourceWrapper<'a>,
}

impl<'a> fmt::Display for IdTaintSource<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl<'a> PartialEq for IdTaintSource<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}

impl<'a> Eq for IdTaintSource<'a> {}

impl<'a> Hash for IdTaintSource<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl<'a> Deref for IdTaintSource<'a> {
    type Target = TaintSource;
    fn deref(&self) -> &Self::Target {
        self.source.deref()
    }
}

/// What a run will actually analyze
///
/// The methods asked for, plus whatever the call graph reached when seeding is on.
#[derive(Default)]
struct WorkPlan<'a> {
    methods: Vec<MethodSpec>,
    seeds: HashMap<MethodId, Vec<IdTaintSource<'a>>>,
    origins: HashMap<MethodId, UnresolvedOrigin>,
}

impl<'a> WorkPlan<'a> {
    fn sources_for(&self, method: &MethodSpec) -> &'a [IdTaintSource<'_>] {
        self.seeds.get(&method.id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn origin_for(&self, method: &MethodSpec) -> UnresolvedOrigin {
        self.origins
            .get(&method.id)
            .cloned()
            .unwrap_or(UnresolvedOrigin::Direct)
    }

    // TODO: I'm not sure the reasoning is sound here, consider:
    //
    // Searching for Activity.getIntent and we asked for it directly, great, but why would that
    // preclude searching for later calls to Activity.getIntent in the graph? I don't see any reason
    // it should. I think this might need to be reworked because that might end up missing a lot of
    // helper methods and the like, but I'm in the middle of a big refactor right now so this has to
    // wait and I'm not even sure what I just wrote is correct. Hi future me, has it been 1 year? 10
    // years? 100 years?

    /// Add a method the call graph reached, merging into what is already planned for it
    ///
    /// This won't add the method if it is already being analyzed directly
    fn add_indirect(&mut self, method: MethodSpec, source: IdTaintSource<'a>) {
        match self.origins.entry(method.id) {
            Entry::Occupied(existing) => {
                // UnresolvedOrigin::Direct means this method is already being analyzed directly,
                // and Indirect means we already planned it. Either way there is nothing to add.
                _ = existing;
                return;
            }
            Entry::Vacant(slot) => {
                // We found a completely new method to analyze as an entrypoint
                slot.insert(UnresolvedOrigin::Indirect);
                self.methods.push(method.clone());
            }
        }

        // May have discovered more sources? I think?
        //
        // Compared by value rather than with `contains`: [IdTaintSource] equality is by id, so
        // the same search arriving under a different id would be pushed again.
        let sources = self.seeds.entry(method.id).or_default();
        if !sources.iter().any(|it| **it == *source) {
            sources.push(source);
        }
    }
}

struct AnalysisSpec {
    analysis_id: AnalyzedMethodId,
    method: MethodSpec,
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
    graph_seeded: bool,
    max_depth: usize,
    cancel: &'a TaskCancelCheck,
    loader: Arc<SsaClassLoader>,
    cache: Arc<WorkerCache<'a>>,
    resolver: Arc<Resolver<'a>>,
    factories: &'a IdFactories,
    tx: &'a Sender<DatabaseAction>,
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
    fn plan(
        &self,
        methods: Vec<MethodSpec>,
        writer: &mut RunWriter,
        db: &TaintAnalysisDbWriter,
    ) -> anyhow::Result<WorkPlan<'_>> {
        let mut plan = WorkPlan::default();

        // Don't include methods that we've already finished on in the plan.
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

            let sources = self.seeds.sources_for(&method);
            let mut method_sources = Vec::with_capacity(sources.len());

            for source in sources {
                let id = writer.id_for_source(source)?;

                method_sources.push(IdTaintSource {
                    id,
                    source: TaintSourceWrapper::from(source),
                });
            }

            plan.seeds.insert(method.id, method_sources);
            // These are all direct calls
            plan.origins.insert(method.id, UnresolvedOrigin::Direct);
            plan.methods.push(method);
        }

        // The existence of `seed` means "yes do seeding via the graph"
        let Some(opts) = self.opts.seed.as_ref() else {
            return Ok(plan);
        };

        // Get all of our method database IDs for some queries
        let entries = plan.methods.iter().map(|it| it.id).collect::<Vec<_>>();

        // Coolect seeds to make them unique and cut down on work

        let mut seen = HashSet::new();
        let sources = plan
            .seeds
            .values()
            .flatten()
            .filter(|it| seen.insert(TaintSource::clone(it)))
            .cloned()
            .collect::<Vec<_>>();

        for source in sources {
            for method in self.reaching_methods(db.graph(), &source, &entries, opts) {
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
                plan.add_indirect(method, source.clone());
            }
        }

        Ok(plan)
    }

    /// Every method reachable from `entries` that this source would seed on
    ///
    /// This is the core action for seeding, see [TaintSeedOptions] for what seeding means
    fn reaching_methods(
        &self,
        gdb: &dyn GraphDatabase,
        source: &TaintSource,
        entries: &[MethodId],
        opts: &TaintSeedOptions,
    ) -> Vec<MethodSpec> {
        let scopes: Vec<Option<&str>> = if opts.sources.is_empty() {
            vec![None]
        } else {
            opts.sources.iter().map(|it| Some(it.as_str())).collect()
        };

        // A scope only narrows which methods or fields are targets, it never changes the entry
        // set, so every scope's targets are resolved up front and the reachability walk runs
        // once instead of once per scope.
        let mut method_targets = Vec::new();
        let mut field_targets = Vec::new();

        for scope in scopes {
            if let Some(search) = source.method_search(scope) {
                match gdb.get_method_ids(&search) {
                    Ok(ids) => method_targets.extend(ids),
                    Err(e) => log::warn!("failed to resolve methods for {source}: {e}"),
                }
            } else if let Some(search) = source.field_search(scope) {
                match gdb.get_field_ids(&search) {
                    Ok(ids) => field_targets.extend(ids),
                    Err(e) => log::warn!("failed to resolve fields for {source}: {e}"),
                }
            }
            // Skipping params
        }

        method_targets.sort_unstable();
        method_targets.dedup();
        field_targets.sort_unstable();
        field_targets.dedup();

        // Each of these is seeded as an entrypoint of its own. The routes that reach them are left
        // in the graph database, see [UnresolvedOrigin::Indirect].
        let mut found = Vec::new();

        if !method_targets.is_empty() {
            match gdb.find_callers_reachable_from(&method_targets, entries) {
                Ok(methods) => found.extend(methods),
                Err(e) => log::warn!("failed to expand seeds for {source}: {e}"),
            }
        }

        if !field_targets.is_empty() {
            match gdb.find_field_refs_reachable_from(&field_targets, FieldAccessOp::Read, entries) {
                Ok(methods) => found.extend(methods),
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
        db: TaintAnalysisDbWriter,
        meta: &RunMeta,
    ) -> anyhow::Result<TaintAnalysisDbWriter> {
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

        let factories = IdFactories::new(&db)?;
        let mut writer = RunWriter::initialize(&db, &meta.as_insert(), &factories)?;

        let mut plan = self.plan(methods, &mut writer, &db)?;

        let ana_methods = writer.update_plan(plan.methods.iter().map(|it| PlannedMethod {
            method: it.id,
            origin: plan.origin_for(it),
        }))?;

        let mut all_methods = std::mem::take(&mut plan.methods)
            .into_iter()
            .zip(ana_methods.into_iter())
            .map(|(method, analysis_id)| AnalysisSpec {
                analysis_id,
                method,
            });

        let gdb = db.graph();

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

        // TODO: Arbitrary
        let (result_tx, result_rx) = bounded::<DatabaseAction>(512);

        thread::scope(|s| {
            // TODO failures on `record*` functions..?
            s.spawn(|| {
                'outer: while let Some(method) = all_methods.next() {
                    if self.cancel.was_cancelled() {
                        break;
                    }
                    loop {
                        select! {
                            send(work_tx, method) -> _ => {
                                break;
                            },
                            recv(fail_rx) -> res => {
                                if let Ok((method, reason)) = res {
                                    if let Err(e) = record_failure(&mut writer, method, reason) {
                                        log::error!("failed to record .. well a failure: {e})");
                                    }
                                }
                            },
                            recv(result_rx) -> res => {
                                if let Ok(action) = res {
                                    if let Err(e) = writer.on_action(action) {
                                        log::error!("failed to handle database action: {e}");
                                    }
                                }
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
                            if let Ok(action) = res {
                                if let Err(e) = writer.on_action(action) {
                                    log::error!("failed to handle database action: {e}");
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

            let cache = Arc::new(WorkerCache::new(&factories.nodes));
            let resolver = Arc::new(Resolver::new(gdb));
            let graph_seeded = self.opts.seed.is_some();

            worker_pool.broadcast(|_| {
                let analyzer = TaintAnalyzerWorker {
                    graph_seeded,
                    ctx: self.ctx,
                    max_depth: self.opts.depth,
                    cancel: &self.cancel,
                    loader: Arc::clone(&self.loader),
                    cache: Arc::clone(&cache),
                    resolver: Arc::clone(&resolver),
                    factories: &factories,
                    tx: &result_tx,
                };

                loop {
                    let Ok(Some(method)) = cancelable_recv(analyzer.cancel, &work_rx) else {
                        break;
                    };

                    let AnalysisSpec {
                        analysis_id,
                        method,
                    } = method;

                    let sources = plan.sources_for(&method);
                    match analyzer.run(analysis_id, &method, sources) {
                        Err(e) => {
                            log::debug!("failed to analyze method {method}: {e}");
                            _ = cancelable_send(analyzer.cancel, (method, e.to_string()), &fail_tx);
                        }
                        Ok(()) => {
                            _ = cancelable_send(
                                analyzer.cancel,
                                DatabaseAction::FinishAnalysis { id: analysis_id },
                                &result_tx,
                            );
                        }
                    }
                }
            });

            drop(fail_tx);
            drop(result_tx);
        });

        writer.finish_run()?;

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
            writer.mark_completed()?;
        }

        Ok(db)
    }
}

fn record_failure(
    writer: &mut RunWriter,
    method: MethodSpec,
    reason: String,
) -> anyhow::Result<()> {
    writer.mark_failed(method.id, &reason)?;
    Ok(())
}

type TaintSet = HashSet<TaintedValue>;
type WorkItem = (Option<NodeId>, TaintedValue);
type WorkQueue = Vec<WorkItem>;

/// Every value produced by a field read in the method, keyed by class and name.
///
/// Seeding a [TaintSource::Field] and propagating out of a field write both want the same answer,
/// and finding it by scanning is linear in the size of the method. There is no aliasing information
/// here: two objects of the same class share an entry.
type FieldReadIndex<'m> = HashMap<(&'m SmaliClassName, &'m str), Vec<ValueId>>;

struct RunState<'b, 'm> {
    analysis_id: AnalyzedMethodId,
    seen: TaintSet,
    field_reads: FieldReadIndex<'b>,
    work: WorkQueue,
    method: &'b MethodSpec,
    ssa_method: &'b SsaMethod<'b>,

    call_stack: &'b mut Vec<MethodId>,

    sources: &'b [IdTaintSource<'b>],
    worker: &'b TaintAnalyzerWorker<'m>,
}

impl<'b, 'm> Deref for RunState<'b, 'm> {
    type Target = TaintAnalyzerWorker<'m>;
    fn deref(&self) -> &Self::Target {
        &self.worker
    }
}

impl<'b, 'm> RunState<'b, 'm> {
    fn new(
        analysis_id: AnalyzedMethodId,
        sources: &'b [IdTaintSource<'b>],
        worker: &'b TaintAnalyzerWorker<'m>,
        method: &'b MethodSpec,
        ssa_method: &'b SsaMethod<'b>,
        call_stack: &'b mut Vec<MethodId>,
    ) -> Self {
        Self {
            analysis_id,
            sources,
            worker,
            method,
            ssa_method,
            call_stack,
            seen: TaintSet::default(),
            field_reads: index_field_reads(ssa_method),
            work: WorkQueue::default(),
        }
    }

    fn run(mut self, sources: &[IdTaintSource], entry: Option<NodeId>) -> anyhow::Result<bool> {
        let mut truncated = false;
        self.seed(sources, entry)?;
        // We have, uh hopefully, queued up some tainted values and now work through them by
        // pushing/popping, creating a depth first traversal of the taint graph.
        while let Some((parent, tv)) = self.pop() {
            if self.cancel.was_cancelled() {
                break;
            }

            let users = self.ssa_method.users(tv.value);

            // Nothing consumes this value, so the path ends here, we don't have to do anything
            // special
            if users.is_empty() {
                continue;
            }

            truncated |= self.on_users(parent, tv, users)?;
        }

        Ok(truncated)
    }

    fn on_field(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        inv: &Invocation,
    ) -> anyhow::Result<()> {
        let Some(fref) = field_ref(inv) else {
            log::warn!("instruction {} had no field target", inv.instruction());
            return Ok(());
        };

        let raw_sink = match self.resolver.resolve_field(fref, self.source()) {
            None => RawSink::ExternalField {
                class: ClassName::from(fref.class),
                name: fref.name.into(),
            },
            Some(id) => RawSink::Field(id),
        };

        let Some(new_parent) = self.push_node(parent, tv, raw_sink, None)? else {
            return Ok(());
        };

        // We treat every read of the field as a source of taint. This isn't technically correct,
        // but we'd have to have dominance to know which ones actually matter. Maybe one day.
        let Some(reads) = self.field_reads.get(&(fref.class, fref.name)) else {
            return Ok(());
        };

        for &value in reads {
            let new_tv = tv.with_value(value);
            if self.seen.insert(new_tv) {
                self.work.push((Some(new_parent), new_tv));
            }
        }

        Ok(())
    }

    /// Mark all values that the provided instruction writes as tainted
    fn taint_defs(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        ins_id: InstructionId,
    ) -> bool {
        let mut added_work = false;
        for &def in self.ssa_method.defs(ins_id) {
            let new_tv = tv.with_value(def);
            added_work |= self.try_add_work(parent, new_tv);
        }
        added_work
    }

    fn on_call(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        ins_id: InstructionId,
        inv: &Invocation,
    ) -> anyhow::Result<bool> {
        let Some(target) = call_target(inv) else {
            log::warn!("instruction {} returned no call target", inv.instruction());
            return Ok(false);
        };

        let call_data = CallData::new(target, self, ins_id, inv, tv.value);

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
                self.taint_defs(parent, tv, ins_id);
            }
            // Return no matter what happens because we never recurse into these types of calls
            return Ok(false);
        }

        // Resolve the callee before recording anything, so the sink can name it by id. A call we
        // can't resolve gets recorded as external instead: there is no id for a method that isn't
        // in the graph database.
        let callee = self.resolver.resolve_method(target, &self.method.source);

        // We need to map the values used to parameter number. For all calls of interest, this is a
        // 1:1 mapping: used[0] -> p0, used[1] -> p1, used[N] -> pN inside the invoked method. This
        // holds true whether the call is static or not.
        //
        // Note also that the tainted value can enter via more than one parameter.
        let used_values = self.ssa_method.uses(ins_id);

        let registers = if used_values.len() == 0 {
            None
        } else {
            let mut registers = Vec::with_capacity(used_values.len());

            for (idx, value) in used_values.iter().copied().enumerate() {
                if value == tv.value {
                    registers.push(idx as RegisterNumber);
                }
            }

            Some(registers)
        };

        // Make sure to propagate the tainted parameters before doing anything

        if call_data.returns_interesting {
            self.taint_defs(parent, tv, ins_id);
        }

        // Handle the case where the call should taint the receiver's value. We do this for <init>
        // and special cased receivers that are invoked fluently
        if call_data.is_init || call_data.special_mutator {
            if let Some(receiver) = call_data.receiver {
                if receiver != tv.value && !self.is_this(receiver) {
                    self.try_add_work(parent, tv.with_value(receiver));
                }
            }
        }

        let callee_ref = callee.as_ref().map(Arc::as_ref);

        // Do a recursion check here before going any further
        if let Some(v) = callee_ref {
            let id = v.get_id();
            if self.call_stack.contains(&id) {
                return Ok(false);
            }
        }

        // Next we attempt to claim the method. Note that we currently can't claim an unresolved
        // method, which makes sense since we won't descend into it anyway.
        let (claim, raw_sink) = match callee_ref {
            // None means external, so we can't get a claim but we can create a sink
            None => (
                None,
                RawSink::ExternalCall {
                    class: ClassName::from(target.class),
                    name: target.name.into(),
                    signature: target.args.into(),
                },
            ),
            Some(v) => {
                let method = v.get_id();
                let cache_key = AnalyzedMethod {
                    method,
                    registers: registers
                        .as_ref()
                        .map(|it| it.clone())
                        .unwrap_or_else(|| Vec::new()),
                };

                let claim = self
                    .cache
                    .claim(cache_key, self.max_depth - self.call_stack.len());

                (Some(claim), RawSink::Call(method))
            }
        };

        // Now we need to decide if this worker should handle this method or if that was sufficient.
        // We'll try to get a claim on the method and only go forward if we get one.

        let truncated = match claim {
            None => {
                // None here just means we have an external call . We _can't_ descend into external
                // calls, so this just makes a new node and we move on with our lives. This node
                // will be deduplicated by the database code so while it's a bit wasteful here it's
                // fine..
                self.push_node(parent, tv, raw_sink, registers)?;
                false
            }
            Some((node, None)) => {
                // Something else has the claim to this method so we just push an edge
                self.push_edge_or_graph(parent, node, tv)?;
                false
            }
            Some((node, Some(mut cookie))) => {
                // We got the claim, so we create the node and handle descent. Note that this could
                // just be because we had higher depth: we have to check the cookie for whether a
                // new node needs to be created or not.

                if cookie.is_new() {
                    if !self.push_node_with_id(parent, node, tv, raw_sink, registers)? {
                        return Ok(false);
                    }
                } else {
                    if !self.push_edge_or_graph(parent, node, tv)? {
                        return Ok(false);
                    }
                }

                let truncated = self.descend(Some(node), tv, ins_id, call_data, callee);

                if truncated {
                    cookie.truncate();
                }
                self.cache.record(cookie);
                truncated
            }
        };
        Ok(truncated)
    }

    /// Attempt to descend into the provided method
    ///
    /// Returns a boolean specifying whether or not the call graph was truncated
    fn descend(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        ins_id: InstructionId,
        call_data: CallData,
        callee: Option<Arc<ResolvedMethod>>,
    ) -> bool {
        // Special mutators are terminal: we don't want to do anything else with them, tainting the
        // receiver was enough. Note that `call_data.is_init` is NOT terminal: we are interested in
        // constructors in general.
        if call_data.special_mutator {
            return false;
        }

        // External _can't_ go any further here
        let Some(resolved_method) = callee else {
            return false;
        };
        // Terminal _shouldn't_ go any further here
        let ResolvedMethod::Inspectable(method) = resolved_method.as_ref() else {
            return false;
        };

        // Don't try to go into abstract/native methods
        if !method.has_body() {
            return false;
        }

        // Don't go deeper than the provided limit
        if self.call_stack.len() >= self.max_depth {
            return true;
        }

        // Upon entering a new function call, if we have `p0` tainted and it is not static, we run
        // into a ton of noise because now pretty much everything becomes tainted. We don't want
        // that, so we don't taint `this` on calls.
        let has_receiver = call_data.receiver.is_some();

        let mut taint_sources = Vec::new();
        // This is a loop because our tainted value can appear more than once
        for (idx, &used) in self.ssa_method.uses(ins_id).iter().enumerate() {
            // As stated above, drop idx 0 if it's non-static, otherwise drop all parameters that
            // are not our tainted value.
            if used != tv.value || (idx == 0 && has_receiver) {
                continue;
            }
            // TODO: We're putting a fake source ID here.. there should probably be a better way to
            // do this for the recursion
            taint_sources.push(IdTaintSource {
                id: TaintSourceId::default(),
                source: TaintSource::Param {
                    register: idx as u16,
                }
                .into(),
            });
        }

        // If the search wasn't seeded at creation time via the graph, we should include
        // non-parameter taint sources: we don't want to lose the information that method
        // calls/field reads are sources of taint.

        // TODO This should probably be mentioned somewhere in the docs, because it will cause
        // descent into every call. Honestly I don't get why seeding is optional in the first place
        // now that I think of it? Ugh.

        if !self.graph_seeded {
            for source in self.sources {
                if source.is_param() {
                    continue;
                }
                taint_sources.push(source.clone());
            }
        }

        if taint_sources.is_empty() {
            return false;
        }

        log::trace!(
            "Recursing into {}->{}({}) in {}",
            method.class,
            method.name,
            method.signature,
            method.source,
        );

        self.call_stack.push(method.id);
        let res = self.worker.run_method(
            self.analysis_id,
            method,
            &taint_sources,
            self.call_stack,
            parent,
        );
        self.call_stack.pop();

        match res {
            Err(e) => {
                log::debug!(
                    "failed to analyze {}->{}({}): {}",
                    method.class,
                    method.name,
                    method.signature,
                    e,
                );
                true
            }
            Ok(v) => v,
        }
    }

    /// Follow a tainted value into every one of its users.
    fn on_users(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        users: &'_ [ValueUser],
    ) -> anyhow::Result<bool> {
        let mut truncated = false;
        for &user in users {
            match user {
                ValueUser::Phi(phi) => {
                    self.push_phi(parent, tv, phi)?;
                }
                ValueUser::Instruction(ins_id) => {
                    let inv = self.ssa_method.instruction(ins_id);
                    if inv.is_call() {
                        truncated |= self.on_call(parent, tv, ins_id, inv)?;
                        continue;
                    }

                    if inv.sets_field() {
                        self.on_field(parent, tv, inv)?;
                        continue;
                    }

                    if inv.sets_array_element() {
                        self.push_array(parent, tv, ins_id)?;
                        continue;
                    }

                    self.on_generic_instruction(parent, tv, ins_id, inv)?;
                }
            }
        }

        Ok(truncated)
    }

    fn source(&self) -> &'b str {
        &self.method.source
    }

    fn try_add_work(&mut self, parent: Option<NodeId>, tv: TaintedValue) -> bool {
        if self.seen.insert(tv) {
            self.work.push((parent, tv));
            true
        } else {
            false
        }
    }

    fn push_new_graph(&self, node: NodeId, source: TaintSourceId) -> anyhow::Result<bool> {
        let graph_id = self.factories.new_graph_id();
        let action = DatabaseAction::StartGraph {
            id: graph_id,
            entry: node,
            analyzed_method: self.analysis_id,
            source,
        };

        self.send(action)
    }

    fn push_edge(&self, src: NodeId, dst: NodeId) -> anyhow::Result<bool> {
        // `dst` was reached from inside the method this run is analyzing, which is the one fact
        // about its location that is true no matter which graph later shares the node
        let action = DatabaseAction::AddEdge {
            src,
            dst,
            location: self.method.id,
        };
        self.send(action)
    }

    /// Push either a new graph or an edge depending on the passed [TaintValue]'s source
    ///
    /// Note that this returns Ok(false) on cancellation!
    fn push_edge_or_graph(
        &mut self,
        parent: Option<NodeId>,
        node: NodeId,
        tv: TaintedValue,
    ) -> anyhow::Result<bool> {
        match parent {
            None => {
                // If there is no previous one this is starting a graph
                if !self.push_new_graph(node, tv.source)? {
                    return Ok(false);
                }
            }
            Some(v) => {
                // Otherwise we need to make a new edge
                if !self.push_edge(v, node)? {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// Push a node into the database
    ///
    /// Note that this returns Ok(None) on cancellation!
    fn push_node(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        sink: RawSink,
        registers: Option<Vec<RegisterNumber>>,
    ) -> anyhow::Result<Option<NodeId>> {
        let (node, insert_action) = self.new_node(sink, registers);
        // Unconditionally push the node, even if there is no graph
        if !self.send(insert_action)? {
            return Ok(None);
        }

        if !self.push_edge_or_graph(parent, node, tv)? {
            Ok(None)
        } else {
            Ok(Some(node))
        }
    }

    fn push_node_with_id(
        &mut self,
        parent: Option<NodeId>,
        id: NodeId,
        tv: TaintedValue,
        sink: RawSink,
        registers: Option<Vec<RegisterNumber>>,
    ) -> anyhow::Result<bool> {
        let insert_action = self.new_node_with_id(id, sink, registers);
        // Unconditionally push the node, even if there is no graph
        if !self.send(insert_action)? {
            return Ok(false);
        }

        self.push_edge_or_graph(parent, id, tv)
    }

    fn new_node_with_id(
        &self,
        id: NodeId,
        sink: RawSink,
        registers: Option<Vec<RegisterNumber>>,
    ) -> DatabaseAction {
        DatabaseAction::AddNode {
            id,
            sink,
            registers,
        }
    }

    fn new_node(
        &self,
        sink: RawSink,
        registers: Option<Vec<RegisterNumber>>,
    ) -> (NodeId, DatabaseAction) {
        let id = self.factories.new_node_id();
        (id, self.new_node_with_id(id, sink, registers))
    }

    fn on_generic_instruction(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        ins_id: InstructionId,
        inv: &Invocation,
    ) -> anyhow::Result<()> {
        // Note that since we do move propagation and the like over in smalisa simple moves are
        // never added here. These are going to be things like throwing exceptions or casting etc
        let Some(new_parent) =
            self.push_node(parent, tv, RawSink::Instruction(inv.instruction()), None)?
        else {
            return Ok(());
        };

        // We'll consider all outputs from instructions with tainted input to be tainted. I'm not
        // sure this is the best way to handle this but that's ok.
        for &out in self.ssa_method.defs(ins_id) {
            let new_tv = tv.with_value(out);
            self.try_add_work(Some(new_parent), new_tv);
        }

        Ok(())
    }

    fn push_array(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        ins_id: InstructionId,
    ) -> anyhow::Result<()> {
        // Pushing the node before getting the value means that if for some reason we fail to get it
        // the array is an end of a route. That's fine and better information than just dropping it
        let Some(new_parent) = self.push_node(parent, tv, RawSink::Array, None)? else {
            return Ok(());
        };

        // The array itself is always the first argument
        let Some(&val) = self.ssa_method.uses(ins_id).first() else {
            return Ok(());
        };

        // Mark the array as tainted if it isn't already tainted
        let tv = tv.with_value(val);
        self.try_add_work(Some(new_parent), tv);
        Ok(())
    }

    fn push_phi(
        &mut self,
        parent: Option<NodeId>,
        tv: TaintedValue,
        phi: PhiId,
    ) -> anyhow::Result<()> {
        let Some(new_parent) = self.push_node(parent, tv, RawSink::Phi, None)? else {
            return Ok(());
        };
        let value = self.ssa_method.phi(phi).value;
        self.try_add_work(Some(new_parent), tv.with_value(value));
        Ok(())
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

    fn pop(&mut self) -> Option<WorkItem> {
        self.work.pop()
    }

    /// Search through the provided [SsaMethod] for all registered [TaintSource]s and add them
    /// to a list of initial tainted values.
    fn seed(&mut self, sources: &[IdTaintSource], entry: Option<NodeId>) -> anyhow::Result<()> {
        for source in sources {
            match &source.source.deref() {
                TaintSource::Field { class, name } => {
                    let tmp = class.get_smali_name();
                    let smali_class = SmaliClassName::from_raw(&tmp);
                    if let Some(reads) = self.field_reads.get(&(&smali_class, name.as_str())) {
                        self.seen
                            .extend(reads.iter().map(|it| TaintedValue::new(source.id, *it)));
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
                            .map(|(idx, _)| TaintedValue::new(source.id, ValueId::new(idx))),
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
                                target.return_type.as_smali_str().as_ref() == it.as_str()
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
                                        .map(|it| TaintedValue::new(source.id, *it)),
                                )
                            }
                        }
                    }
                }
            }
        }

        for tv in &self.seen {
            self.work.push((entry, *tv));
        }

        Ok(())
    }
}

#[derive(Eq, PartialEq, Hash, Ord, PartialOrd, Debug, Clone)]
#[repr(transparent)]
struct FieldKey(String);

impl FieldKey {
    fn new() -> Self {
        Self(String::with_capacity(256))
    }

    fn set(&mut self, fref: &FieldRef, source: &str) {
        self.0.clear();
        let ty = &fref.ty;
        let smali_ty = ty.as_smali_str();

        for part in [fref.class.as_str(), &smali_ty, source] {
            self.0.push_str(part);
            // NULL can't appear in valid smali so it's a good sep here, otherwise we could
            // accidentally get weirdness
            self.0.push('\0');
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
        let smali_ty = ty.as_smali_str();

        for part in [mref.class, mref.name, mref.args, &smali_ty, source] {
            self.0.push_str(part);
            // NULL can't appear in valid smali so it's a good sep here, otherwise we could
            // accidentally get weirdness
            self.0.push('\0');
        }
    }
}

struct Resolver<'a> {
    gdb: &'a dyn GraphDatabase,

    // The current implementation of the resolvers requires them to be wrapped in Mutexs.
    // Technically we could put their caches in their own mutex and scope the locks more precisely,
    // but the way we make keys would also have to change or be behind its own lock. This is just
    // easier for now.
    fields: Mutex<FieldResolver>,
    methods: Mutex<MethodResolver>,
}

impl<'a> Resolver<'a> {
    fn new(gdb: &'a dyn GraphDatabase) -> Self {
        let methods = Mutex::new(MethodResolver::new());
        let fields = Mutex::new(FieldResolver::new());
        Self {
            gdb,
            fields,
            methods,
        }
    }

    fn resolve_field(&self, target: &FieldRef, source: &str) -> Option<FieldId> {
        self.fields
            .lock()
            .expect("poisoned mutex")
            .resolve(self.gdb, target, source)
    }

    fn resolve_method(&self, target: &MethodRef, source: &str) -> Option<Arc<ResolvedMethod>> {
        self.methods
            .lock()
            .expect("poisoned mutex")
            .resolve(self.gdb, target, source)
    }
}

struct FieldResolver {
    cache: LruCache<FieldKey, Option<FieldId>>,
    stats: CacheStats,
    key: FieldKey,
}

impl FieldResolver {
    fn new() -> Self {
        let cache = LruCache::new(NonZeroUsize::new(1024).unwrap());
        Self {
            cache,
            key: FieldKey::new(),
            stats: CacheStats::new("fields"),
        }
    }

    fn resolve(
        &mut self,
        gdb: &dyn GraphDatabase,
        target: &FieldRef,
        source: &str,
    ) -> Option<FieldId> {
        self.stats.lookup_attempt();
        self.key.set(target, source);

        if let Some(cached) = self.cache.get(&self.key) {
            self.stats.lru_hit();
            return *cached;
        };

        let class = ClassName::from(target.class);

        match gdb.get_field_source_or_framework(&class, target.name, source) {
            Ok(Some(v)) => {
                let field = v.id;
                self.cache.put(self.key.clone(), Some(field));
                Some(field)
            }
            Ok(None) => {
                self.cache.put(self.key.clone(), None);
                log::warn!("field {}->{}: not in database", target.class, target.name);
                None
            }
            Err(e) => {
                log::error!(
                    "failed to find the field {}->{}: {}",
                    target.class,
                    target.name,
                    e
                );
                None
            }
        }
    }
}

struct MethodResolver {
    cache: LruCache<MethodKey, Option<Arc<ResolvedMethod>>>,
    stats: CacheStats,
    key: MethodKey,
}

impl MethodResolver {
    fn new() -> Self {
        let cache = LruCache::new(NonZeroUsize::new(1024).unwrap());
        Self {
            cache,
            key: MethodKey::new(),
            stats: CacheStats::new("methods"),
        }
    }

    fn resolve(
        &mut self,
        gdb: &dyn GraphDatabase,
        target: &MethodRef,
        source: &str,
    ) -> Option<Arc<ResolvedMethod>> {
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

        self.stats.lookup_attempt();

        self.key.set(target, source);

        if let Some(cached) = self.cache.get(&self.key) {
            self.stats.lru_hit();
            return cached.as_ref().map(Arc::clone);
        };

        let class = ClassName::from(target.class);

        let return_type = target.return_type.as_smali_str();

        match gdb.get_method_source_or_framework(
            &class,
            target.name,
            target.args,
            &return_type,
            source,
        ) {
            Ok(Some(v)) => {
                let is_terminal = TERMINAL_NAMESPACES
                    .iter()
                    .any(|it| target.class.starts_with(it));

                let resolved = Arc::new(if is_terminal {
                    ResolvedMethod::Terminal(v)
                } else {
                    ResolvedMethod::Inspectable(v)
                });
                self.cache
                    .put(self.key.clone(), Some(Arc::clone(&resolved)));
                Some(resolved)
            }
            Ok(None) => {
                self.cache.put(self.key.clone(), None);
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
        id: AnalyzedMethodId,
        method: &MethodSpec,
        sources: &[IdTaintSource],
    ) -> anyhow::Result<()> {
        let mut call_stack = Vec::new();
        _ = self.run_method(id, method, sources, &mut call_stack, None)?;
        Ok(())
    }

    /// Run analysis on the provided method with the provided sources
    ///
    /// Returns a boolean indicating whether the call graph was truncated or not
    fn run_method(
        &self,
        id: AnalyzedMethodId,
        method: &MethodSpec,
        sources: &[IdTaintSource<'_>],
        call_stack: &mut Vec<MethodId>,
        entry: Option<NodeId>,
    ) -> anyhow::Result<bool> {
        log::debug!("Starting {} on method {method}", sources.iter().join(", "));
        let class =
            self.loader
                .get_ssa_class(self.ctx, method.class_id, &method.class, &method.source)?;
        let class = class.get();

        let Some(ssa_method) = get_ssa_method(class, method) else {
            bail!("failed to find method {}", method);
        };

        let state = RunState::new(id, sources, self, method, &ssa_method, call_stack);
        let res = state.run(sources, entry);
        log::debug!("Done with {} on method {method}", sources.iter().join(", "));
        res
    }

    /// Attempt to send the [DatabaseAction], returns false if cancelled
    fn send(&self, action: DatabaseAction) -> anyhow::Result<bool> {
        let sent = cancelable_send(self.cancel, action, self.tx)?;
        Ok(sent)
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
            "Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)",
            "*->getIntent()",
            "*->getIntent()Landroid/content/Intent;",
            "Landroid/content/Intent;->getExtras()",
            "Landroid/app/Activity;->getIntent()Landroid/content/Intent;",
            "Lcom/example/Thing;->path",
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
