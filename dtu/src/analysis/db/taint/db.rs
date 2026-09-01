use std::collections::{HashMap, HashSet};
use std::fmt::{self, Debug};
use std::hash::Hash;
use std::mem::discriminant;
use std::ops::Deref;
use std::rc::Rc;
use std::str::FromStr;

use anyhow::bail;
use bitflags::bitflags;
use diesel::expression::ValidGrouping;
use diesel::query_builder::{AstPass, QueryFragment, QueryId};
use diesel::r2d2::CustomizeConnection;
use diesel::sql_types::{BigInt, Bool, Integer, Nullable, Text};
use diesel::sqlite::Sqlite;
use diesel::{delete, insert_or_ignore_into, prelude::*, r2d2};
use diesel::{sql_query, SqliteConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations};
use serde::Serialize;
use smalisa::AccessFlag;

use crate::analysis::db::taint::models::{
    AnalyzedMethod, AnalyzedMethodId, ExternalSinkId, FieldSinkId, InsertHiddenAnalyzedMethod,
    InsertHiddenRoute, InstructionSinkId, MethodStatus, RouteId, SinkId, SinkKind, SourceKind,
    TaintSourceId,
};
use crate::analysis::db::taint::schema::{
    _hidden_analyzed_methods, _hidden_routes, call_graph_chains, external_sinks, field_sinks,
    instruction_sinks, routes, sink_filters, sinks, source_calls, source_fields, source_params,
    taint_sources,
};
use crate::analysis::taint::TaintSource;
use crate::db::common::apply_connection_pragmas;
use crate::db::graph::db::MethodSpecRow;
use crate::db::graph::schema::{classes, methods, sources};
use crate::db::graph::MethodSpec;
use crate::db::{query_exec, DatabaseId};
use crate::utils::{ClassName, Container};
use crate::{
    analysis::db::taint::schema::{analyzed_methods, run_info},
    db::{
        self,
        common::Db,
        graph::{
            db::GRAPH_DATABASE_FILE_NAME, models::MethodId, GraphDatabase, GraphSqliteDatabase,
        },
        query,
    },
    utils::path_must_str,
    Context, Version, VERSION,
};
use yoke::Yokeable;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/taint_migrations/");

#[derive(Clone, Copy)]
pub enum TaintDatabaseValidity {
    Consistent,
    MalformedMeta,
    OlderThanGraph,
    GraphHasDeletes,
    IncompatibleGraphVersion,
    OldSchema,
}

/// The chains a [UnresolvedOrigin::CallGraph] carries come from the call graph, not from the
/// dataflow analysis: they say the method is reachable, not that anything tainted flows along them.
#[derive(Debug, Clone, PartialEq)]
pub enum UnresolvedOrigin {
    /// The method was asked for directly
    Direct,
    /// The call graph led here from a method that was asked for
    ///
    /// Each chain runs from the method that was asked for to this one, naming methods by id the
    /// same way sinks do
    CallGraph { chains: Vec<Vec<MethodId>> },
}

pub enum ResolvedOrigin<'a> {
    /// The method was asked for directly
    Direct,
    /// The call graph led here from a method that was asked for
    ///
    /// Each chain runs from the method that was asked for to this one, naming methods by id the
    /// same way sinks do
    CallGraph {
        chains: Vec<Vec<&'a MethodSpecAndDisplay>>,
    },
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Serialize)]
pub enum SinkDef {
    Field {
        class: ClassName,
        name: String,
    },
    Instruction(String),
    External {
        class: ClassName,
        name: String,
        signature: String,
    },
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Serialize)]
pub struct SinkDefAndDisplay {
    pub sink_def: SinkDef,
    pub display: String,
}

impl fmt::Display for SinkDefAndDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

impl AsRef<str> for SinkDefAndDisplay {
    fn as_ref(&self) -> &str {
        &self.display
    }
}

impl Deref for SinkDefAndDisplay {
    type Target = SinkDef;
    fn deref(&self) -> &Self::Target {
        &self.sink_def
    }
}

impl AsRef<SinkDef> for SinkDefAndDisplay {
    fn as_ref(&self) -> &SinkDef {
        &self.sink_def
    }
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Serialize)]
pub struct TaintSourceAndDisplay {
    pub source: TaintSource,
    pub display: String,
}

impl Deref for TaintSourceAndDisplay {
    type Target = TaintSource;
    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

impl AsRef<TaintSource> for TaintSourceAndDisplay {
    fn as_ref(&self) -> &TaintSource {
        &self.source
    }
}

impl fmt::Display for TaintSourceAndDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

impl AsRef<str> for TaintSourceAndDisplay {
    fn as_ref(&self) -> &str {
        &self.display
    }
}

bitflags! {
    /// Used to select which IDs should be resolvable in a [ReportIdMap]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ResolvableIds: u8 {
        const Methods = 1 << 0;
        const Sinks = 1 << 1;
        const Sources = 1 << 2;
        const All = Self::Methods.bits() | Self::Sinks.bits() | Self::Sources.bits();
    }
}

impl ResolvableIds {
    fn includes_methods(self) -> bool {
        self.contains(Self::Methods)
    }

    fn includes_sources(self) -> bool {
        self.contains(Self::Sources)
    }

    fn includes_sinks(self) -> bool {
        self.contains(Self::Sinks)
    }
}

fn sinks_serialize<S>(
    sinks: &Option<HashMap<SinkId, SinkDefAndDisplay>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let fixup: Option<HashMap<i32, &SinkDefAndDisplay>> = sinks
        .as_ref()
        .map(|it| HashMap::from_iter(it.iter().map(|(k, v)| (k.raw(), v))));
    fixup.serialize(serializer)
}

/// Used to translate IDs into concrete types for a given analyssi run
#[derive(Serialize)]
pub struct ReportIdMap {
    pub methods: Option<HashMap<MethodId, MethodSpecAndDisplay>>,
    #[serde(serialize_with = "sinks_serialize")]
    pub sinks: Option<HashMap<SinkId, SinkDefAndDisplay>>,
    pub sources: Option<HashMap<TaintSourceId, TaintSourceAndDisplay>>,
}

#[derive(thiserror::Error, Debug)]
pub enum IdLookupError {
    #[error("id map doesn't contain {kind:?} id {id}")]
    Missing { kind: ResolvableIds, id: i32 },
    #[error("id map wasn't built to resolve kind {kind:?}")]
    Unresolved { kind: ResolvableIds },
}

impl ReportIdMap {
    fn get<K, V>(
        map: &Option<HashMap<K, V>>,
        id: K,
        kind: ResolvableIds,
    ) -> Result<&V, IdLookupError>
    where
        K: Hash + Eq + Debug + Into<i32>,
    {
        map.as_ref()
            .ok_or(IdLookupError::Unresolved { kind })
            .and_then(|it| {
                it.get(&id).ok_or(IdLookupError::Missing {
                    kind,
                    id: id.into(),
                })
            })
    }

    pub fn get_method(&self, id: MethodId) -> Result<&MethodSpecAndDisplay, IdLookupError> {
        Self::get(&self.methods, id, ResolvableIds::Methods)
    }

    pub fn get_method_spec(&self, id: MethodId) -> Result<&MethodSpec, IdLookupError> {
        self.get_method(id).map(|it| &it.spec)
    }

    pub fn get_method_display(&self, id: MethodId) -> Result<&str, IdLookupError> {
        self.get_method(id).map(|it| it.display.as_str())
    }

    pub fn get_sink<T: Into<SinkId>>(&self, sink: T) -> Result<&SinkDefAndDisplay, IdLookupError> {
        let sink = sink.into();
        Self::get(&self.sinks, sink, ResolvableIds::Sinks)
    }

    pub fn get_source(
        &self,
        source: TaintSourceId,
    ) -> Result<&TaintSourceAndDisplay, IdLookupError> {
        Self::get(&self.sources, source, ResolvableIds::Sources)
    }
}

impl TaintDatabaseValidity {
    pub fn is_valid(self) -> bool {
        matches!(self, Self::Consistent)
    }

    pub fn to_error(self) -> anyhow::Result<()> {
        match self {
            Self::MalformedMeta => {
                bail!("The taint results database has malformed metadata");
            }
            Self::OlderThanGraph => {
                bail!("Graph database is newer than the reference, stale IDs may be present");
            }
            Self::GraphHasDeletes => {
                bail!(
                "Graph database has deletes after taint report created, stale IDs may be present"
            );
            }
            Self::IncompatibleGraphVersion => {
                bail!("Database and graph have incompatible versions");
            }

            Self::OldSchema => {
                bail!(
                    "Database uses an old schema version, current is {}",
                    TaintAnalysisDb::SCHEMA_VERSION
                );
            }

            Self::Consistent => {}
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct TaintAnalysisDb {
    pub(super) db: Db,
}

impl Deref for TaintAnalysisDb {
    type Target = Db;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

#[derive(Debug)]
struct GraphSqliteAttach {
    path: String,
}

impl CustomizeConnection<SqliteConnection, r2d2::Error> for GraphSqliteAttach {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), r2d2::Error> {
        apply_connection_pragmas(conn)?;
        let path = &self.path;

        // Try to attach with the URI and mode=ro first
        let uri_res = query!(sql_query(format!(
            r#"ATTACH DATABASE 'file:{path}?mode=ro' AS graph"#
        )))
        .execute(conn);

        match uri_res {
            Ok(_) => Ok(()),
            Err(_) => {
                // Fallback to reguar attach and pragma
                query!(sql_query(format!(
                    r#"ATTACH DATABASE '{path}' AS graph; PRAGMA graph.query_only = true;"#
                )))
                .execute(conn)?;
                Ok(())
            }
        }
    }
}

/// A [TaintAnalysisDb] with the the graph database attached
///
/// Use [Self::graph] to get a [GraphDatabase] implementation
#[derive(Clone)]
pub struct GraphTaintAnalysisDb {
    db: TaintAnalysisDb,
    graph: GraphSqliteDatabase,
}

impl Deref for GraphTaintAnalysisDb {
    type Target = TaintAnalysisDb;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

diesel::table! {
    sink_filters_fts(rowid) {
        rowid -> Integer,
    }
}

diesel::allow_tables_to_appear_in_same_query!(sink_filters, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(sinks, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(taint_sources, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(routes, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(analyzed_methods, sink_filters_fts);

diesel::joinable!(
    analyzed_methods -> methods (method)
);
diesel::joinable!(
    call_graph_chains -> methods (method)
);

diesel::joinable!(
    sinks -> methods (location)
);

macro_rules! can_methodspec {
    ($table:ident) => {
        diesel::allow_tables_to_appear_in_same_query!($table, methods);
        diesel::allow_tables_to_appear_in_same_query!($table, sources);
        diesel::allow_tables_to_appear_in_same_query!($table, classes);
    };
}

can_methodspec!(analyzed_methods);
can_methodspec!(call_graph_chains);
can_methodspec!(sinks);
can_methodspec!(taint_sources);
can_methodspec!(routes);

/// A [MethodSpec] with the smali form attached for display
#[derive(Clone, Debug, Serialize)]
pub struct MethodSpecAndDisplay {
    pub spec: MethodSpec,
    pub display: String,
}

impl Deref for MethodSpecAndDisplay {
    type Target = MethodSpec;
    fn deref(&self) -> &Self::Target {
        &self.spec
    }
}

impl fmt::Display for MethodSpecAndDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

impl AsRef<str> for MethodSpecAndDisplay {
    fn as_ref(&self) -> &str {
        &self.display
    }
}

pub struct AnalyzedMethodSpec {
    pub analysis_id: AnalyzedMethodId,
    pub nchains: usize,
    pub nroutes: usize,
    pub spec: MethodSpec,
    pub status: MethodStatus,
    pub error: Option<String>,
}

impl Deref for AnalyzedMethodSpec {
    type Target = MethodSpec;
    fn deref(&self) -> &Self::Target {
        &self.spec
    }
}

#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub enum UnresolvedTaintSinkKind {
    Phi,
    Array,
    Call(MethodId),
    ExternalCall(ExternalSinkId),
    Field(FieldSinkId),
    Instruction(InstructionSinkId),
}

pub struct UnresolvedTaintSink {
    pub in_method: MethodId,
    pub sink: UnresolvedTaintSinkKind,
}

#[derive(Serialize, Debug)]
pub enum ResolvedTaintSinkKind<'a> {
    Phi,
    Array,
    Call(&'a MethodSpecAndDisplay),
    Instruction(&'a str),
    ExternalCall(&'a str),
    Field(&'a str),
}

impl<'a> AsRef<str> for ResolvedTaintSinkKind<'a> {
    fn as_ref(&self) -> &str {
        match self {
            Self::Phi => "<Phi(...)>",
            Self::Array => "<Array>",
            Self::Call(it) => &it.display,
            Self::Instruction(it) | Self::ExternalCall(it) | Self::Field(it) => it,
        }
    }
}

impl<'a> fmt::Display for ResolvedTaintSinkKind<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl<'a> ResolvedTaintSinkKind<'a> {
    fn resolve(
        unresolved: UnresolvedTaintSinkKind,
        map: &'a ReportIdMap,
    ) -> Result<Self, ResolveError> {
        type USink = UnresolvedTaintSinkKind;

        Ok(match unresolved {
            USink::Array => Self::Array,
            USink::Phi => Self::Phi,
            USink::Call(id) => {
                let method = map.get_method(id)?;
                Self::Call(method)
            }
            USink::Instruction(id) => {
                let sink = map.get_sink(id)?;
                Self::Instruction(&sink.display)
            }
            USink::ExternalCall(id) => {
                let sink = map.get_sink(id)?;
                Self::ExternalCall(&sink.display)
            }

            USink::Field(id) => {
                let sink = map.get_sink(id)?;
                Self::Field(&sink.display)
            }
        })
    }
}

#[derive(Serialize, Debug)]
pub struct ResolvedTaintSink<'a> {
    pub sink: ResolvedTaintSinkKind<'a>,
    pub in_method: &'a MethodSpecAndDisplay,
}

#[derive(thiserror::Error, Debug)]
pub enum ResolveError {
    #[error("lookup failed during resolution: {0}")]
    IdLookup(IdLookupError),
}

impl From<IdLookupError> for ResolveError {
    fn from(value: IdLookupError) -> Self {
        Self::IdLookup(value)
    }
}

#[derive(Clone, Debug)]
pub enum Filter {
    NoPhi,
    ClassContains(String),
    MethodContains(String),
    FieldContains(String),
    RetContains(String),
    SigContains(String),
    MinLength(usize),
    MaxLength(usize),
}

impl Filter {
    pub fn matches(&self, route: &ResolvedTaintRoute) -> bool {
        match self {
            Self::NoPhi => route.phis == 0,
            Self::MinLength(len) => route.sinks.len() >= *len,
            Self::MaxLength(len) => route.sinks.len() <= *len,

            Self::FieldContains(needle) => route.sinks.iter().any(|sink| match sink.sink {
                ResolvedTaintSinkKind::Phi
                | ResolvedTaintSinkKind::Array
                | ResolvedTaintSinkKind::Call(_)
                | ResolvedTaintSinkKind::ExternalCall(_)
                | ResolvedTaintSinkKind::Instruction(_) => false,
                ResolvedTaintSinkKind::Field(f) => f
                    .split_once("->")
                    .is_some_and(|(_, field)| field.contains(needle)),
            }),

            Self::RetContains(needle) => route.sinks.iter().any(|sink| match sink.sink {
                ResolvedTaintSinkKind::Phi
                | ResolvedTaintSinkKind::Array
                | ResolvedTaintSinkKind::Field(_)
                | ResolvedTaintSinkKind::Instruction(_) => false,
                ResolvedTaintSinkKind::Call(m) => m.spec.ret.as_str().contains(needle),
                ResolvedTaintSinkKind::ExternalCall(s) => s
                    .rsplit_once(')')
                    .is_some_and(|(_, ret)| ret.contains(needle)),
            }),

            Self::SigContains(needle) => route.sinks.iter().any(|sink| match sink.sink {
                ResolvedTaintSinkKind::Phi
                | ResolvedTaintSinkKind::Array
                | ResolvedTaintSinkKind::Field(_)
                | ResolvedTaintSinkKind::Instruction(_) => false,
                ResolvedTaintSinkKind::Call(m) => m.spec.signature.as_str().contains(needle),
                ResolvedTaintSinkKind::ExternalCall(s) => {
                    s.rsplit_once('(').is_some_and(|(_, rem)| {
                        rem.split_once(')')
                            .is_some_and(|(args, _)| args.contains(needle))
                    })
                }
            }),

            Self::MethodContains(needle) => route.sinks.iter().any(|sink| match sink.sink {
                ResolvedTaintSinkKind::Phi
                | ResolvedTaintSinkKind::Array
                | ResolvedTaintSinkKind::Field(_)
                | ResolvedTaintSinkKind::Instruction(_) => false,
                ResolvedTaintSinkKind::Call(m) => m.spec.name.as_str().contains(needle),
                ResolvedTaintSinkKind::ExternalCall(s) => {
                    s.split_once("->").is_some_and(|(_, rem)| {
                        rem.split_once('(')
                            .is_some_and(|(method, _)| method.contains(needle))
                    })
                }
            }),
            Self::ClassContains(needle) => route.sinks.iter().any(|sink| match sink.sink {
                ResolvedTaintSinkKind::Phi
                | ResolvedTaintSinkKind::Array
                | ResolvedTaintSinkKind::Instruction(_) => false,
                ResolvedTaintSinkKind::Call(m) => m.spec.class.as_str().contains(needle),
                ResolvedTaintSinkKind::ExternalCall(s) | ResolvedTaintSinkKind::Field(s) => s
                    .split_once("->")
                    .is_some_and(|(cls, _)| cls.contains(needle)),
            }),
        }
    }

    fn parse_len(param: &str, value: Option<&str>) -> anyhow::Result<usize> {
        let parsed = value
            .ok_or_else(|| anyhow::Error::msg(format!("need a value for {param}")))
            .and_then(|it| match it.parse::<usize>() {
                Ok(v) => Ok(v),
                Err(_) => Err(anyhow::Error::msg(format!(
                    "invalid length for {param}: {it}"
                ))),
            })?;
        Ok(parsed)
    }
}

impl FromStr for Filter {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let (name, value) = match s.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (s, None),
        };
        Ok(match name {
            "min-len" => Self::MinLength(Self::parse_len(name, value)?),
            "max-len" => Self::MaxLength(Self::parse_len(name, value)?),
            "nophi" => Self::NoPhi,

            "sig" => {
                let m = value.ok_or_else(|| anyhow::Error::msg("sig needs a value"))?;
                Self::SigContains(m.into())
            }

            "ret" => {
                let m = value.ok_or_else(|| anyhow::Error::msg("ret needs a value"))?;
                Self::RetContains(m.into())
            }

            "method" => {
                let m = value.ok_or_else(|| anyhow::Error::msg("method needs a value"))?;
                Self::MethodContains(m.into())
            }

            "field" => {
                let m = value.ok_or_else(|| anyhow::Error::msg("field needs a value"))?;
                Self::FieldContains(m.into())
            }

            "class" => {
                let m = value.ok_or_else(|| anyhow::Error::msg("class needs a value"))?;
                Self::ClassContains(m.into())
            }
            _ => bail!("invalid filter option: {name}"),
        })
    }
}

impl UnresolvedTaintSink {
    pub fn resolve<'a>(&self, map: &'a ReportIdMap) -> Result<ResolvedTaintSink<'a>, ResolveError> {
        let in_method = map.get_method(self.in_method)?;
        let sink = ResolvedTaintSinkKind::resolve(self.sink, map)?;
        Ok(ResolvedTaintSink { sink, in_method })
    }
}

pub struct UnresolvedTaintRoutes {
    pub routes: Vec<UnresolvedTaintRoute>,
    /// If this is [UnresolvedOrigin::CallGraph], it is the chains until the start method. This
    /// happens when the taint analysis was run with seeding. Note that this is multi dimensional
    /// because there can be multiple paths to the analyzed method. This list is complete as far as
    /// the graph database is concerned.
    pub origin: UnresolvedOrigin,
}

impl Deref for UnresolvedTaintRoutes {
    type Target = Vec<UnresolvedTaintRoute>;
    fn deref(&self) -> &Self::Target {
        &self.routes
    }
}

/// The taint routes in a report completely unresolved; use a [ReportIdMap] to resolve as needed or
/// use [Self::resolve] to resolve it.
pub struct UnresolvedTaintRoute {
    pub id: RouteId,
    pub incomplete: bool,
    pub phis: usize,
    pub source: TaintSourceId,
    pub sinks: Vec<UnresolvedTaintSink>,
}

#[derive(Serialize, Debug)]
pub struct ResolvedTaintRoute<'a> {
    pub id: RouteId,
    pub incomplete: bool,
    pub phis: usize,
    pub source: &'a TaintSourceAndDisplay,
    pub sinks: Vec<ResolvedTaintSink<'a>>,
}

impl<'a> ResolvedTaintRoute<'a> {
    fn resolve(
        unresolved: &UnresolvedTaintRoute,
        map: &'a ReportIdMap,
    ) -> Result<Self, ResolveError> {
        let UnresolvedTaintRoute {
            id,
            incomplete,
            phis,
            sinks,
            source,
        } = unresolved;

        let source = map.get_source(*source)?;

        let mut resolved_sinks = Vec::with_capacity(sinks.len());

        for sink in sinks {
            let resolved = sink.resolve(map)?;
            resolved_sinks.push(resolved);
        }

        Ok(ResolvedTaintRoute {
            id: *id,
            incomplete: *incomplete,
            phis: *phis,
            source,
            sinks: resolved_sinks,
        })
    }
}

impl UnresolvedTaintRoute {
    pub fn resolve<'a>(
        &self,
        map: &'a ReportIdMap,
    ) -> Result<ResolvedTaintRoute<'a>, ResolveError> {
        ResolvedTaintRoute::resolve(self, map)
    }
}

#[derive(Yokeable)]
pub struct ResolvedTaintRoutes<'a> {
    pub routes: Vec<ResolvedTaintRoute<'a>>,
    pub origin: ResolvedOrigin<'a>,
}

impl<'a> Deref for ResolvedTaintRoutes<'a> {
    type Target = Vec<ResolvedTaintRoute<'a>>;
    fn deref(&self) -> &Self::Target {
        &self.routes
    }
}

pub struct YokedResolvedTaintRoutes(yoke::Yoke<ResolvedTaintRoutes<'static>, Rc<ReportIdMap>>);

impl Container for YokedResolvedTaintRoutes {
    type Item<'a> = ResolvedTaintRoute<'a>;
    fn len(&self) -> usize {
        self.0.get().len()
    }
    fn get_item<'a>(&'a self, index: usize) -> &'a Self::Item<'a> {
        &self.0.get().routes[index]
    }
}

pub struct ResolvedTaintRoutesIter<'a> {
    routes: &'a [ResolvedTaintRoute<'a>],
    at: usize,
}

impl<'a> Iterator for ResolvedTaintRoutesIter<'a> {
    type Item = &'a ResolvedTaintRoute<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let route = self.routes.get(self.at)?;
        self.at += 1;
        Some(route)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.routes.len() - self.at;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for ResolvedTaintRoutesIter<'a> {}

impl YokedResolvedTaintRoutes {
    pub fn iter(&self) -> ResolvedTaintRoutesIter<'_> {
        let routes = &self.0.get().routes;
        ResolvedTaintRoutesIter { routes, at: 0 }
    }

    pub fn get<'a>(&'a self) -> &'a ResolvedTaintRoutes<'a> {
        self.0.get()
    }
}

impl ResolvedTaintRoutes<'_> {
    pub fn resolve<'a>(
        routes: UnresolvedTaintRoutes,
        map: &'a ReportIdMap,
    ) -> Result<ResolvedTaintRoutes<'a>, ResolveError> {
        let mut resolved = Vec::new();
        for route in routes.routes {
            resolved.push(ResolvedTaintRoute::resolve(&route, map)?);
        }

        let origin = match routes.origin {
            UnresolvedOrigin::Direct => ResolvedOrigin::Direct,
            UnresolvedOrigin::CallGraph {
                chains: unresolved_chains,
            } => {
                let mut chains = Vec::new();
                for chain in unresolved_chains {
                    // Shouldn't happen, right?
                    if chain.len() == 0 {
                        continue;
                    }
                    let mut resolved_chain = Vec::new();
                    for method in chain {
                        let resolved = map.get_method(method)?;
                        resolved_chain.push(resolved);
                    }
                    chains.push(resolved_chain);
                }

                ResolvedOrigin::CallGraph { chains }
            }
        };

        Ok(ResolvedTaintRoutes {
            routes: resolved,
            origin,
        })
    }

    /// Resolve into a yoked [ResolvedTaintRoute] with the [ReportIdMap] attached as the cart. This
    /// takes ownership of the map, but the map can still be used via the cart.
    pub fn resolve_yoked(
        routes: UnresolvedTaintRoutes,
        map: ReportIdMap,
    ) -> Result<YokedResolvedTaintRoutes, ResolveError> {
        let rc_map = Rc::new(map);

        yoke::Yoke::try_attach_to_cart(rc_map, |map| Self::resolve(routes, map))
            .map(YokedResolvedTaintRoutes)
    }
}

impl GraphTaintAnalysisDb {
    pub fn new_from_path<S: AsRef<str> + ?Sized>(ctx: &dyn Context, path: &S) -> db::Result<Self> {
        let graph_path = ctx.get_sqlite_dir()?.join(GRAPH_DATABASE_FILE_NAME);
        let graph_path_str = path_must_str(&graph_path);

        let db = Db::new_from_path_with(
            path,
            MIGRATIONS,
            #[cfg(test)]
            MIGRATIONS,
            Some(Box::new(GraphSqliteAttach {
                path: graph_path_str.into(),
            })),
        )?;

        let taint_db = TaintAnalysisDb { db: db.clone() };
        let graph = GraphSqliteDatabase::wrap(db);

        Ok(Self {
            db: taint_db,
            graph,
        })
    }

    /// Check if the database is valid
    ///
    /// This will make sure that the provided database is capable of being consistent with the given
    /// [Context]'s graph database.
    pub fn get_validity(&self) -> db::Result<TaintDatabaseValidity> {
        let (version, reference_graph_built_at, sversion) = self.query(|c| {
            query!(run_info::table.select((
                run_info::dtu_version,
                run_info::graph_built_at,
                run_info::schema_version
            )))
            .get_result::<(String, i64, i32)>(c)
        })?;

        if sversion != TaintAnalysisDb::SCHEMA_VERSION {
            return Ok(TaintDatabaseValidity::OldSchema);
        }

        let Some(version) = Version::from_full(&version) else {
            return Ok(TaintDatabaseValidity::MalformedMeta);
        };

        if !version.major_matches(VERSION) {
            return Ok(TaintDatabaseValidity::IncompatibleGraphVersion);
        }

        let gdb = self.graph();

        let current_graph_built_at = gdb.get_built_at()?;
        let last_delete = gdb.get_last_delete()?;

        if current_graph_built_at > reference_graph_built_at {
            return Ok(TaintDatabaseValidity::OlderThanGraph);
        };

        if last_delete.is_some_and(|it| it > reference_graph_built_at) {
            return Ok(TaintDatabaseValidity::GraphHasDeletes);
        }

        Ok(TaintDatabaseValidity::Consistent)
    }

    pub fn get_validity_err(&self) -> anyhow::Result<()> {
        self.get_validity()?.to_error()
    }

    pub fn graph(&self) -> &impl GraphDatabase {
        &self.graph
    }

    pub fn get_analysis_matching(
        &self,
        filters: &[Filter],
    ) -> anyhow::Result<Vec<AnalyzedMethodId>> {
        // This is currently a bit of a mess... Duplicate filter kinds should be ORs not silently
        // dropped. The queries can be made more efficient and more obvious. I dunno. A lot to do
        // probably.

        if filters.len() == 0 {
            return Ok(self.query(|c| {
                query!(analyzed_methods::table.select(analyzed_methods::id)).get_results(c)
            })?);
        }

        let filter_fields = filters
            .iter()
            .any(|it| matches!(it, Filter::FieldContains(_)));
        if filter_fields {
            let filter_methods = filters.iter().any(|it| {
                matches!(
                    it,
                    Filter::MethodContains(_) | Filter::RetContains(_) | Filter::SigContains(_)
                )
            });

            if filter_methods {
                bail!("filtering on fields and methods will always return nothing");
            }
        }

        let fts5 = self.db.db.check_fts5();

        if fts5 {
            return self.get_analysis_matching_fts5(filters, filter_fields);
        }
        self.get_analysis_matching_no_fts5(filters)
    }

    fn get_analysis_matching_fts5(
        &self,
        filters: &[Filter],
        filter_fields: bool,
    ) -> anyhow::Result<Vec<AnalyzedMethodId>> {
        let mut seen_kinds = HashSet::new();
        let mut fts5_queries = Vec::new();
        let mut no_phi = false;
        let mut min_length = None;
        let mut max_length = None;

        for filter in filters {
            if !seen_kinds.insert(discriminant(filter)) {
                continue;
            }
            match filter {
                Filter::SigContains(s) => {
                    fts5_queries.push(format!("args:{s}"));
                }
                Filter::ClassContains(s) => {
                    fts5_queries.push(format!("class:{s}"));
                }
                Filter::MethodContains(s) | Filter::FieldContains(s) => {
                    fts5_queries.push(format!("name:{s}"));
                }
                Filter::RetContains(s) => {
                    fts5_queries.push(format!("ret:{s}"));
                }
                Filter::NoPhi => {
                    no_phi = true;
                }
                Filter::MinLength(len) => {
                    min_length = Some(*len);
                }
                Filter::MaxLength(len) => {
                    max_length = Some(*len);
                }
            }
        }

        let kind_condition = if filter_fields {
            "sink_filters.kind = 'field'"
        } else {
            "sink_filters.kind != 'field'"
        };

        let fts_condition = if fts5_queries.is_empty() {
            ""
        } else {
            "AND sink_filters_fts MATCH ?"
        };

        let no_phi_condition = if no_phi {
            "AND routes.phi_count = 0"
        } else {
            ""
        };

        let min_length_condition = if min_length.is_some() {
            r#"
        AND (
            SELECT COUNT(*)
            FROM sinks AS min_sinks
            WHERE min_sinks.route = routes.id
        ) >= ?
        "#
        } else {
            ""
        };

        let max_length_condition = if max_length.is_some() {
            r#"
        AND (
            SELECT COUNT(*)
            FROM sinks AS max_sinks
            WHERE max_sinks.route = routes.id
        ) < ?
        "#
        } else {
            ""
        };

        let sql = format!(
            r#"
        SELECT DISTINCT taint_sources.analyzed_method AS id
        FROM sink_filters_fts
        INNER JOIN sink_filters
            ON sink_filters.rowid = sink_filters_fts.rowid
        INNER JOIN sinks
            ON sinks.route = sink_filters.route
           AND sinks.idx = sink_filters.idx
        INNER JOIN routes
            ON routes.id = sinks.route
        INNER JOIN taint_sources
            ON taint_sources.id = routes.source
        WHERE {kind_condition}
        {fts_condition}
        {no_phi_condition}
        {min_length_condition}
        {max_length_condition}
        "#
        );

        let mut query = sql_query(sql).into_boxed();

        if !fts5_queries.is_empty() {
            query = query.bind::<Text, _>(fts5_queries.join(" AND "));
        }

        if let Some(len) = min_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        if let Some(len) = max_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        Ok(self.query(|conn| query!(query).get_results::<AnalyzedMethodId>(conn))?)
    }

    fn get_analysis_matching_no_fts5(
        &self,
        filters: &[Filter],
    ) -> anyhow::Result<Vec<AnalyzedMethodId>> {
        let mut query = analyzed_methods::table
            .select(analyzed_methods::id)
            .into_boxed();

        let mut seen_kinds = HashSet::new();

        for filter in filters {
            if !seen_kinds.insert(discriminant(filter)) {
                continue;
            }

            match filter {
                Filter::NoPhi => {
                    query = query.filter(diesel::dsl::exists(
                        taint_sources::table
                            .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                            .filter(taint_sources::analyzed_method.eq(analyzed_methods::id))
                            .filter(routes::phi_count.eq(0)),
                    ));
                }
                Filter::MinLength(len) => {
                    query = query.filter(diesel::dsl::exists(
                        taint_sources::table
                            .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                            .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                            .filter(taint_sources::analyzed_method.eq(analyzed_methods::id))
                            .group_by(routes::id)
                            .having(diesel::dsl::count(sinks::idx).ge(*len as i64)),
                    ));
                }
                Filter::MaxLength(len) => {
                    query = query.filter(diesel::dsl::exists(
                        taint_sources::table
                            .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                            .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                            .filter(taint_sources::analyzed_method.eq(analyzed_methods::id))
                            .group_by(routes::id)
                            .having(diesel::dsl::count(sinks::idx).lt(*len as i64)),
                    ));
                }
                _ => bail!("text search requires FTS5"),
            }
        }

        Ok(self.query(|c| query!(query).get_results(c))?)
    }

    pub fn get_routes_matching(
        &self,
        analysis_id: AnalyzedMethodId,
        filters: &[Filter],
    ) -> anyhow::Result<Vec<RouteId>> {
        // See comments above analysis IDs

        if filters.len() == 0 {
            return Ok(self.query(|c| {
                query!(analyzed_methods::table
                    .inner_join(taint_sources::table)
                    .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                    .select(routes::id)
                    .filter(analyzed_methods::id.eq(analysis_id)))
                .get_results(c)
            })?);
        }

        let filter_fields = filters
            .iter()
            .any(|it| matches!(it, Filter::FieldContains(_)));
        if filter_fields {
            let filter_methods = filters.iter().any(|it| {
                matches!(
                    it,
                    Filter::MethodContains(_) | Filter::RetContains(_) | Filter::SigContains(_)
                )
            });

            if filter_methods {
                bail!("filtering on fields and methods will all");
            }
        }

        let fts5 = self.db.db.check_fts5();

        if fts5 {
            return self.get_routes_matching_fts5(analysis_id, filters, filter_fields);
        }
        self.get_routes_matching_no_fts5(analysis_id, filters)
    }

    fn get_routes_matching_fts5(
        &self,
        analysis_id: AnalyzedMethodId,
        filters: &[Filter],
        filter_fields: bool,
    ) -> anyhow::Result<Vec<RouteId>> {
        let mut seen_kinds = HashSet::new();

        let mut fts5_queries = Vec::new();
        let mut no_phi = false;
        let mut min_length = None;
        let mut max_length = None;

        for filter in filters {
            if !seen_kinds.insert(discriminant(filter)) {
                continue;
            }

            match filter {
                Filter::SigContains(s) => {
                    fts5_queries.push(format!("args:{s}"));
                }
                Filter::ClassContains(s) => {
                    fts5_queries.push(format!("class:{s}"));
                }
                Filter::MethodContains(s) | Filter::FieldContains(s) => {
                    fts5_queries.push(format!("name:{s}"));
                }
                Filter::RetContains(s) => {
                    fts5_queries.push(format!("ret:{s}"));
                }
                Filter::NoPhi => {
                    no_phi = true;
                }
                Filter::MinLength(len) => {
                    min_length = Some(*len);
                }
                Filter::MaxLength(len) => {
                    max_length = Some(*len);
                }
            }
        }

        let kind_condition = if filter_fields {
            "sink_filters.kind = 'field'"
        } else {
            "sink_filters.kind != 'field'"
        };

        let fts_condition = if fts5_queries.is_empty() {
            ""
        } else {
            "AND sink_filters_fts MATCH ?"
        };

        let no_phi_condition = if no_phi {
            "AND routes.phi_count = 0"
        } else {
            ""
        };

        let min_length_condition = if min_length.is_some() {
            r#"
        AND (
            SELECT COUNT(*)
            FROM sinks AS min_sinks
            WHERE min_sinks.route = routes.id
        ) >= ?
        "#
        } else {
            ""
        };

        let max_length_condition = if max_length.is_some() {
            r#"
        AND (
            SELECT COUNT(*)
            FROM sinks AS max_sinks
            WHERE max_sinks.route = routes.id
        ) < ?
        "#
        } else {
            ""
        };

        let sql = format!(
            r#"
        SELECT DISTINCT routes.id AS id
        FROM sink_filters_fts
        INNER JOIN sink_filters
            ON sink_filters.rowid = sink_filters_fts.rowid
        INNER JOIN sinks
            ON sinks.route = sink_filters.route
           AND sinks.idx = sink_filters.idx
        INNER JOIN routes
            ON routes.id = sinks.route
        INNER JOIN taint_sources
            ON taint_sources.id = routes.source
        WHERE taint_sources.analyzed_method = ?
          AND {kind_condition}
          {fts_condition}
          {no_phi_condition}
          {min_length_condition}
          {max_length_condition}
        "#
        );

        let mut query = sql_query(sql).into_boxed();

        query = query.bind::<Integer, _>(analysis_id.raw());

        if !fts5_queries.is_empty() {
            query = query.bind::<Text, _>(fts5_queries.join(" AND "));
        }

        if let Some(len) = min_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        if let Some(len) = max_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        Ok(self.query(|conn| query!(query).get_results::<RouteId>(conn))?)
    }

    fn get_routes_matching_no_fts5(
        &self,
        analysis_id: AnalyzedMethodId,
        filters: &[Filter],
    ) -> anyhow::Result<Vec<RouteId>> {
        let mut query = analyzed_methods::table
            .inner_join(taint_sources::table)
            .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
            .select(routes::id)
            .filter(analyzed_methods::id.eq(analysis_id))
            .into_boxed();

        let mut seen_kinds = HashSet::new();

        for filter in filters {
            if !seen_kinds.insert(discriminant(filter)) {
                continue;
            }

            match filter {
                Filter::NoPhi => {
                    query = query.filter(routes::phi_count.eq(0));
                }
                Filter::MinLength(len) => {
                    query = query.filter(diesel::dsl::exists(
                        sinks::table
                            .filter(sinks::route.eq(routes::id))
                            .group_by(sinks::route)
                            .having(diesel::dsl::count(sinks::idx).ge(*len as i64)),
                    ));
                }
                Filter::MaxLength(len) => {
                    query = query.filter(diesel::dsl::exists(
                        sinks::table
                            .filter(sinks::route.eq(routes::id))
                            .group_by(sinks::route)
                            .having(diesel::dsl::count(sinks::idx).lt(*len as i64)),
                    ));
                }
                _ => bail!("text search requires FTS5"),
            }
        }

        Ok(self.query(|c| query!(query).get_results(c))?)
    }

    /// Get the [AnalyzedMethod] for the given [AnalyzedMethodId]
    pub fn get_analysis_info(&self, id: AnalyzedMethodId) -> db::Result<AnalyzedMethod> {
        self.db.query(|c| {
            query!(analyzed_methods::table
                .select(AnalyzedMethod::as_select())
                .filter(analyzed_methods::id.eq(id)))
            .get_result::<AnalyzedMethod>(c)
        })
    }

    /// Get the [AnalyzedMethod] entry for the given [MethodId]
    pub fn get_analysis_for_method(&self, method_id: MethodId) -> db::Result<AnalyzedMethod> {
        self.db.query(|c| {
            query!(analyzed_methods::table
                .select(AnalyzedMethod::as_select())
                .filter(analyzed_methods::method.eq(method_id)))
            .get_result::<AnalyzedMethod>(c)
        })
    }

    /// Get the [MethodId] for the given [AnalyzedMethodId]
    pub fn get_method_for_analysis(&self, id: AnalyzedMethodId) -> db::Result<MethodId> {
        self.db.query(|c| {
            query!(analyzed_methods::table
                .select(analyzed_methods::method)
                .filter(analyzed_methods::id.eq(id)))
            .get_result::<MethodId>(c)
        })
    }

    fn get_sources_map(
        &self,
        ana: Option<&AnalyzedMethod>,
    ) -> db::Result<HashMap<TaintSourceId, TaintSourceAndDisplay>> {
        let id = ana.map(|it| it.id);

        self.query(
            |c| -> db::Result<HashMap<TaintSourceId, TaintSourceAndDisplay>> {
                let mut map: HashMap<TaintSourceId, TaintSourceAndDisplay> = HashMap::new();

                map.extend(
                    query!(source_params::table
                        .inner_join(taint_sources::table)
                        .filter(taint_sources::kind.eq(SourceKind::Param))
                        .filter(OptionalId::new(taint_sources::analyzed_method, id))
                        .select((taint_sources::id, source_params::register))
                        .distinct())
                    .get_results::<(TaintSourceId, i32)>(c)?
                    .into_iter()
                    .map(|(id, reg)| {
                        (
                            id,
                            TaintSourceAndDisplay {
                                source: TaintSource::Param {
                                    register: reg as u16,
                                },
                                display: format!("p{reg}"),
                            },
                        )
                    }),
                );

                map.extend(
                    query!(source_fields::table
                        .inner_join(taint_sources::table)
                        .filter(taint_sources::kind.eq(SourceKind::Field))
                        .filter(OptionalId::new(taint_sources::analyzed_method, id))
                        .select((taint_sources::id, source_fields::class, source_fields::name))
                        .distinct())
                    .get_results::<(TaintSourceId, ClassName, String)>(c)?
                    .into_iter()
                    .map(|(id, class, name)| {
                        let display = format!("{class};->{name}");
                        (
                            id,
                            TaintSourceAndDisplay {
                                source: TaintSource::Field { class, name },
                                display,
                            },
                        )
                    }),
                );

                map.extend(
                    query!(source_calls::table
                        .inner_join(taint_sources::table)
                        .filter(taint_sources::kind.eq(SourceKind::Call))
                        .filter(OptionalId::new(taint_sources::analyzed_method, id))
                        .select((
                            taint_sources::id,
                            source_calls::class,
                            source_calls::name,
                            source_calls::args,
                            source_calls::ret,
                        ))
                        .distinct())
                    .get_results::<(
                        TaintSourceId,
                        Option<ClassName>,
                        String,
                        String,
                        Option<String>,
                    )>(c)?
                    .into_iter()
                    .map(|(id, class, method, args, ret)| {
                        let display = format!(
                            "{};->{}({}){}",
                            class.as_ref().map(|it| it.as_ref()).unwrap_or("*"),
                            method,
                            args,
                            ret.as_ref().map(|it| it.as_str()).unwrap_or("")
                        );
                        (
                            id,
                            TaintSourceAndDisplay {
                                source: TaintSource::MethodCall {
                                    class,
                                    method,
                                    args,
                                    ret,
                                },
                                display,
                            },
                        )
                    }),
                );

                Ok(map)
            },
        )
    }

    fn get_sinks_map(
        &self,
        ana: Option<&AnalyzedMethod>,
    ) -> db::Result<HashMap<SinkId, SinkDefAndDisplay>> {
        let id = ana.map(|it| it.id);

        self.query(|c| -> db::Result<HashMap<SinkId, SinkDefAndDisplay>> {
            let mut map: HashMap<SinkId, SinkDefAndDisplay> = HashMap::new();

            map.extend(
                query!(analyzed_methods::table
                    .inner_join(taint_sources::table)
                    .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                    .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                    .inner_join(
                        external_sinks::table
                            .on(external_sinks::id.eq(sinks::sink_id.assume_not_null())),
                    )
                    .filter(OptionalId::new(analyzed_methods::id, id))
                    .filter(sinks::kind.eq(SinkKind::External))
                    .select((
                        external_sinks::id,
                        external_sinks::class,
                        external_sinks::name,
                        external_sinks::signature,
                    )))
                .get_results::<(ExternalSinkId, ClassName, String, String)>(c)?
                .into_iter()
                .map(|(id, class, name, signature)| {
                    let display = format!("{class}->{name}({signature})");
                    (
                        SinkId::External(id),
                        SinkDefAndDisplay {
                            sink_def: SinkDef::External {
                                class,
                                name,
                                signature,
                            },
                            display,
                        },
                    )
                }),
            );

            map.extend(
                query!(analyzed_methods::table
                    .inner_join(taint_sources::table)
                    .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                    .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                    .inner_join(
                        field_sinks::table.on(field_sinks::id.eq(sinks::sink_id.assume_not_null())),
                    )
                    .filter(OptionalId::new(analyzed_methods::id, id))
                    .filter(sinks::kind.eq(SinkKind::Field))
                    .select((field_sinks::id, field_sinks::class, field_sinks::name)))
                .get_results::<(FieldSinkId, ClassName, String)>(c)?
                .into_iter()
                .map(|(id, class, name)| {
                    let display = format!("{class}->{name}");
                    (
                        SinkId::Field(id),
                        SinkDefAndDisplay {
                            sink_def: SinkDef::Field { class, name },
                            display,
                        },
                    )
                }),
            );

            map.extend(
                query!(analyzed_methods::table
                    .inner_join(taint_sources::table)
                    .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                    .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                    .inner_join(
                        instruction_sinks::table
                            .on(instruction_sinks::id.eq(sinks::sink_id.assume_not_null())),
                    )
                    .filter(OptionalId::new(analyzed_methods::id, id))
                    .filter(sinks::kind.eq(SinkKind::Instruction))
                    .select((instruction_sinks::id, instruction_sinks::instruction)))
                .get_results::<(InstructionSinkId, String)>(c)?
                .into_iter()
                .map(|(id, ins)| {
                    let display = ins.clone();
                    (
                        SinkId::Instruction(id),
                        SinkDefAndDisplay {
                            sink_def: SinkDef::Instruction(ins),
                            display,
                        },
                    )
                }),
            );

            Ok(map)
        })
    }

    fn get_methods_map(
        &self,
        ana: Option<&AnalyzedMethod>,
    ) -> db::Result<HashMap<MethodId, MethodSpecAndDisplay>> {
        let id = ana.map(|it| it.id);

        // Grab every method referenced by this analyzed method. This includes:
        //  - sinks.location
        //  - sinks.sink_id when sink.kind = 'call'
        //  - call_graph_chains.method
        //  - analyzed_methods.method

        let all_methods = self.query(|c| {
            query!(methods::table
                .inner_join(call_graph_chains::table)
                .inner_join(sources::table)
                .inner_join(classes::table)
                .select(MethodSpecRow::as_select())
                .filter(OptionalId::new(call_graph_chains::analyzed_method, id))
                .distinct()
                .union(
                    taint_sources::table
                        .inner_join(routes::table)
                        .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                        .inner_join(methods::table.on(methods::id.eq(sinks::location)))
                        .inner_join(classes::table.on(classes::id.eq(methods::class)))
                        .inner_join(sources::table.on(sources::id.eq(methods::source)))
                        .select(MethodSpecRow::as_select())
                        .filter(OptionalId::new(taint_sources::analyzed_method, id))
                )
                .union(
                    taint_sources::table
                        .inner_join(routes::table)
                        .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                        .inner_join(
                            methods::table.on(methods::id.eq(sinks::method_id.assume_not_null())),
                        )
                        .inner_join(classes::table.on(classes::id.eq(methods::class)))
                        .inner_join(sources::table.on(sources::id.eq(methods::source)))
                        .select(MethodSpecRow::as_select())
                        .filter(OptionalId::new(taint_sources::analyzed_method, id))
                        .filter(sinks::kind.eq(SinkKind::Call)),
                )
                .union(
                    methods::table
                        .inner_join(analyzed_methods::table)
                        .inner_join(sources::table)
                        .inner_join(classes::table)
                        .select(MethodSpecRow::as_select())
                        .filter(OptionalId::new(analyzed_methods::id, id))
                        .limit(1)
                ))
            .get_results::<MethodSpecRow>(c)
        })?;

        Ok(HashMap::from_iter(all_methods.into_iter().map(|it| {
            let spec = MethodSpec::from(it);
            let display = spec.as_smali();
            (spec.id, MethodSpecAndDisplay { spec, display })
        })))
    }

    fn _get_report_id_map(
        &self,
        ana: Option<&AnalyzedMethod>,
        select: ResolvableIds,
    ) -> db::Result<ReportIdMap> {
        let methods = if select.includes_methods() {
            self.get_methods_map(ana).map(Some)?
        } else {
            None
        };

        let sinks = if select.includes_sinks() {
            self.get_sinks_map(ana).map(Some)?
        } else {
            None
        };

        let sources = if select.includes_sources() {
            self.get_sources_map(ana).map(Some)?
        } else {
            None
        };

        Ok(ReportIdMap {
            methods,
            sinks,
            sources,
        })
    }

    /// Return a [ReportIdMap] that covers the entire database. For a specific analysis run, prefer
    /// [Self::get_report_id_map]
    pub fn get_complete_report_id_map(&self, select: ResolvableIds) -> db::Result<ReportIdMap> {
        self._get_report_id_map(None, select)
    }

    /// Return a [ReportIdMap] for the given analysis run
    pub fn get_report_id_map(
        &self,
        ana: &AnalyzedMethod,
        select: ResolvableIds,
    ) -> db::Result<ReportIdMap> {
        self._get_report_id_map(Some(ana), select)
    }

    /// Get all [UnresolvedTaintRoute]s for the given analysis
    ///
    /// To resolve all of the IDs, use a [ReportIdMap] with [ResolvedTaintRoutes::resolve]
    pub fn get_routes_for_analysis(
        &self,
        ana: &AnalyzedMethod,
    ) -> anyhow::Result<UnresolvedTaintRoutes> {
        let all_sinks = self.db.query(|c| {
            query!(analyzed_methods::table
                .inner_join(taint_sources::table)
                .inner_join(routes::table.on(routes::source.eq(taint_sources::id)))
                .inner_join(sinks::table.on(sinks::route.eq(routes::id)))
                .filter(analyzed_methods::id.eq(ana.id))
                .order_by(sinks::route)
                .then_order_by(sinks::idx)
                .select((
                    taint_sources::id,
                    routes::id,
                    routes::incomplete,
                    routes::phi_count,
                    sinks::kind,
                    sinks::sink_id,
                    sinks::method_id,
                    sinks::location,
                    sinks::idx
                )))
            .get_results::<(
                TaintSourceId,
                RouteId,
                bool,
                i32,
                SinkKind,
                Option<i32>,
                Option<MethodId>,
                MethodId,
                i32,
            )>(c)
        })?;

        let origin = if ana.direct {
            UnresolvedOrigin::Direct
        } else {
            let chains = self.get_chains_for_analysis(ana)?;
            UnresolvedOrigin::CallGraph { chains }
        };

        let mut routes = Vec::new();

        for (taint_source, route_id, incomplete, phis, kind, sink_id, method_id, location, idx) in
            all_sinks
        {
            let sink = match kind {
                SinkKind::Call => match method_id {
                    None => bail!("BUG! method_id NULL for call kind!"),
                    Some(v) => UnresolvedTaintSinkKind::Call(v),
                },
                SinkKind::Phi => UnresolvedTaintSinkKind::Phi,
                SinkKind::Array => UnresolvedTaintSinkKind::Array,
                SinkKind::Field => match sink_id {
                    None => bail!("BUG! sink_id NULL for field kind!"),
                    Some(v) => UnresolvedTaintSinkKind::Field(v.into()),
                },
                SinkKind::Instruction => match sink_id {
                    None => bail!("BUG! sink_id NULL for instruction kind!"),
                    Some(v) => UnresolvedTaintSinkKind::Instruction(v.into()),
                },
                SinkKind::External => match sink_id {
                    None => bail!("BUG! sink_id NULL for external kind!"),
                    Some(v) => UnresolvedTaintSinkKind::ExternalCall(v.into()),
                },
            };

            let sink = UnresolvedTaintSink {
                in_method: location,
                sink,
            };

            if idx == 0 {
                let new = UnresolvedTaintRoute {
                    id: route_id,
                    incomplete,
                    source: taint_source,
                    phis: phis as usize,
                    sinks: vec![sink],
                };
                routes.push(new);
            } else {
                match routes.last_mut() {
                    Some(v) => {
                        v.sinks.push(sink);
                    }
                    None => {
                        let new = UnresolvedTaintRoute {
                            id: route_id,
                            incomplete,
                            source: taint_source,
                            phis: phis as usize,
                            sinks: vec![sink],
                        };
                        routes.push(new);
                    }
                }
            }
        }

        Ok(UnresolvedTaintRoutes { routes, origin })
    }

    pub fn get_chains_for_analysis(&self, ana: &AnalyzedMethod) -> db::Result<Vec<Vec<MethodId>>> {
        if ana.direct {
            return Ok(Vec::new());
        }
        let results = self.query(|c| {
            query!(call_graph_chains::table
                .select((call_graph_chains::method, call_graph_chains::idx))
                .filter(call_graph_chains::analyzed_method.eq(ana.id))
                .order_by(call_graph_chains::chain)
                .then_order_by(call_graph_chains::idx))
            .get_results::<(MethodId, i32)>(c)
        })?;

        let mut chains = Vec::new();
        for (method, idx) in results {
            if idx == 0 {
                chains.push(vec![method]);
            } else {
                let Some(chain) = chains.last_mut() else {
                    chains.push(vec![method]);
                    continue;
                };
                chain.push(method);
            }
        }

        Ok(chains)
    }

    pub fn get_all_analyzed_method_specs(&self) -> db::Result<Vec<AnalyzedMethodSpec>> {
        #[derive(QueryableByName)]
        #[diesel(check_for_backend(Sqlite))]
        pub struct QueryResult {
            #[diesel(sql_type = BigInt)]
            chain_count: i64,
            #[diesel(sql_type = Integer)]
            analyzed_method: i32,
            #[diesel(sql_type = Text)]
            status: MethodStatus,
            #[diesel(sql_type = Nullable<Text>)]
            error: Option<String>,
            #[diesel(sql_type = Integer)]
            route_count: i32,
            #[diesel(sql_type = Integer)]
            class_id: i32,
            #[diesel(sql_type = Text)]
            class: ClassName,
            #[diesel(sql_type = Integer)]
            id: i32,
            #[diesel(sql_type = Text)]
            name: String,
            #[diesel(sql_type = Text)]
            args: String,
            #[diesel(sql_type = Text)]
            ret: String,
            #[diesel(sql_type = BigInt)]
            access_flags: i64,
            #[diesel(sql_type = Text)]
            source: String,
        }

        let rows = self.db.query(|c| {
            query!(sql_query(
                r#"
SELECT  count(DISTINCT cg.chain) AS chain_count,
        am.id           AS analyzed_method,
        am.status       AS status,
        am.error        AS error,
        am.route_count  AS route_count,
        c.id            AS class_id,
        c.name          AS class,
        m.id            AS id,
        m.name          AS name,
        m.args          AS args,
        m.ret           AS ret,
        m.access_flags  AS access_flags,
        s.name          AS source
FROM methods AS m
JOIN analyzed_methods AS am
    ON am.method = m.id
JOIN classes AS c
    ON c.id = m.class
JOIN sources AS s
    ON s.id = m.source
LEFT JOIN call_graph_chains AS cg
    ON cg.analyzed_method = am.id
GROUP BY am.id;"#
            ))
            .get_results::<QueryResult>(c)
        })?;

        Ok(rows
            .into_iter()
            .map(
                |QueryResult {
                     chain_count,
                     analyzed_method,
                     status,
                     error,
                     route_count,
                     class_id,
                     class,
                     id,
                     name,
                     args,
                     ret,
                     access_flags,
                     source,
                 }| AnalyzedMethodSpec {
                    analysis_id: analyzed_method.into(),
                    nchains: chain_count as usize,
                    nroutes: route_count as usize,
                    status,
                    error,
                    spec: MethodSpec {
                        class_id: class_id.into(),
                        class,
                        name,
                        id: id.into(),
                        signature: args,
                        ret,
                        access_flags: AccessFlag::from_bits_truncate(access_flags as u64),
                        source,
                    },
                },
            )
            .collect::<Vec<_>>())
    }
}

impl AnalyzedMethod {
    pub fn ensure_valid(&self) -> anyhow::Result<()> {
        match self.status {
            MethodStatus::Failed => match &self.error {
                Some(v) => bail!("analysis {} failed with error: {}", self.id, v),
                None => bail!("analysis {} failed with an unspecified error", self.id),
            },
            MethodStatus::Pending => {
                bail!("analysis {} is still pending", self.id);
            }
            MethodStatus::Done => Ok(()),
        }
    }
}

impl TaintAnalysisDb {
    pub const SCHEMA_VERSION: i32 = 1;

    pub fn new_from_path<S: AsRef<str> + ?Sized>(path: &S) -> db::Result<Self> {
        Ok(Self {
            db: Db::new_from_path(
                path,
                MIGRATIONS,
                #[cfg(test)]
                MIGRATIONS,
            )?,
        })
    }

    pub fn get_all_analyzed_methods(&self) -> db::Result<Vec<AnalyzedMethod>> {
        Ok(self.query(|c| {
            analyzed_methods::table
                .select(AnalyzedMethod::as_select())
                .get_results::<AnalyzedMethod>(c)
        })?)
    }

    /// Retrieve all methods referenced by the database
    ///
    /// This includes analyzed methods, methods along paths, and call sinks. Note that this can't
    /// include external calls which are by definition not in the database.
    pub fn get_all_method_ids(&self) -> db::Result<Vec<MethodId>> {
        self.query(|c| {
            query!(analyzed_methods::table
                .select(analyzed_methods::method)
                .distinct()
                .union(sinks::table.select(sinks::location))
                .union(
                    sinks::table
                        .select(sinks::sink_id.assume_not_null())
                        .filter(sinks::sink_id.is_not_null())
                        .filter(sinks::kind.eq(SinkKind::Call)),
                ))
            .get_results::<MethodId>(c)
        })
    }

    pub fn unhide_route(&self, route: RouteId) -> db::Result<()> {
        self.write(|c| -> db::Result<()> {
            query_exec!(
                delete(_hidden_routes::table).filter(_hidden_routes::route.eq(route)),
                c
            )?;
            Ok(())
        })
    }

    pub fn unhide_analyzed_method(&self, ana: AnalyzedMethodId) -> db::Result<()> {
        self.write(|c| -> db::Result<()> {
            query_exec!(
                delete(_hidden_analyzed_methods::table)
                    .filter(_hidden_analyzed_methods::analyzed_method.eq(ana)),
                c
            )?;
            Ok(())
        })
    }

    pub fn hide_analyzed_method(&self, ana: AnalyzedMethodId) -> db::Result<()> {
        self.write(|c| -> db::Result<()> {
            let value = InsertHiddenAnalyzedMethod {
                analyzed_method: ana,
            };
            query_exec!(
                insert_or_ignore_into(_hidden_analyzed_methods::table).values(&value),
                c
            )?;
            Ok(())
        })
    }

    pub fn hide_route(&self, route: RouteId) -> db::Result<()> {
        self.write(|c| -> db::Result<()> {
            let value = InsertHiddenRoute { route };
            query_exec!(
                insert_or_ignore_into(_hidden_routes::table).values(&value),
                c
            )?;
            Ok(())
        })
    }

    pub fn get_hidden_routes(&self) -> db::Result<Vec<RouteId>> {
        Ok(self.query(|c| {
            query!(_hidden_routes::table.select(_hidden_routes::route)).get_results::<RouteId>(c)
        })?)
    }

    pub fn get_hidden_analyzed_methods(&self) -> db::Result<Vec<AnalyzedMethodId>> {
        Ok(self.query(|c| {
            query!(_hidden_analyzed_methods::table
                    .select(_hidden_analyzed_methods::analyzed_method))
                .get_results::<AnalyzedMethodId>(c)
        })?)
    }
}

#[derive(Debug, Clone)]
pub struct Fts5Match(String);

impl Fts5Match {
    pub fn new(query: impl Into<String>) -> Self {
        Self(query.into())
    }
}

impl Expression for Fts5Match {
    type SqlType = Bool;
}

impl QueryId for Fts5Match {
    type QueryId = Fts5Match;
    const HAS_STATIC_QUERY_ID: bool = false;
}

impl QueryFragment<Sqlite> for Fts5Match {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        out.push_sql("sink_filters_fts MATCH ");
        out.push_bind_param::<Text, _>(&self.0)?;
        Ok(())
    }
}

impl ValidGrouping<()> for Fts5Match {
    type IsAggregate = diesel::expression::is_aggregate::No;
}

impl<QS> diesel::AppearsOnTable<QS> for Fts5Match {}

#[derive(Debug, Clone, Copy)]
struct OptionalId<C> {
    column: C,
    value: Option<i32>,
}

impl<C> ValidGrouping<()> for OptionalId<C> {
    type IsAggregate = diesel::expression::is_aggregate::No;
}

impl<QS, C> diesel::AppearsOnTable<QS> for OptionalId<C> {}

impl<C> OptionalId<C> {
    fn new<T: DatabaseId>(column: C, value: Option<T>) -> Self {
        Self {
            column,
            value: value.map(|it| it.id()),
        }
    }
}

impl<C> Expression for OptionalId<C> {
    type SqlType = Bool;
}

impl<C> QueryId for OptionalId<C>
where
    C: 'static,
{
    type QueryId = OptionalId<C>;
    const HAS_STATIC_QUERY_ID: bool = false;
}

impl<C> QueryFragment<Sqlite> for OptionalId<C>
where
    C: QueryFragment<Sqlite>,
{
    fn walk_ast<'b>(&'b self, mut pass: AstPass<'_, 'b, Sqlite>) -> QueryResult<()> {
        match &self.value {
            Some(value) => {
                self.column.walk_ast(pass.reborrow())?;
                pass.push_sql(" = ");
                pass.push_bind_param::<Integer, _>(value)?;
            }
            None => {
                pass.push_sql("TRUE");
            }
        }

        Ok(())
    }
}
