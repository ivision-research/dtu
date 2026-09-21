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

use crate::analysis::taint::models::{ExternalFieldSinkId, NodeId, SubgraphId};
use crate::analysis::taint::TaintSource;
use crate::db::common::apply_connection_pragmas;
use crate::db::graph::db::{FieldSpecRow, MethodSpecRow};
use crate::db::graph::models::{FieldId, FieldSpec};
use crate::db::graph::schema::{class_fields, classes, methods, sources};
use crate::db::graph::MethodSpec;
use crate::db::{query_exec, DatabaseId};
use crate::utils::{ClassName, Container};
use crate::{
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

use super::models::{
    AnalyzedMethod, AnalyzedMethodId, ExternalCallSinkId, InsertHiddenAnalyzedMethod,
    InsertHiddenGraph, InstructionSinkId, MethodStatus, SinkId, SinkKind, SourceKind,
    TaintSourceId,
};
use super::schema::{
    _hidden_analyzed_methods, _hidden_graphs, edges, external_call_sinks, external_field_sinks,
    graphs, instruction_sinks, nodes, reachable_nodes, sink_filters, source_calls, source_fields,
    source_params, taint_sources,
};
use super::schema::{analyzed_methods, run_info};

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

#[derive(Debug, Clone, PartialEq)]
pub enum UnresolvedOrigin {
    /// The method was asked for directly
    Direct,
    /// The call graph led here from a method that was asked for
    ///
    /// The route that got here is deliberately not recorded. There may be an enormous number of
    /// them and they are all recoverable from the graph database, so anyone who decides a graph is
    /// worth chasing can ask for the paths then.
    Indirect,
}

pub enum ResolvedOrigin {
    /// The method was asked for directly
    Direct,
    /// The call graph led here from a method that was asked for, see
    /// [UnresolvedOrigin::Indirect]
    Indirect,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Serialize)]
pub enum SinkDef {
    ExternalField {
        class: ClassName,
        name: String,
    },
    Instruction(String),
    ExternalCall {
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
        const Fields = 1 << 3;
        const All = Self::Methods.bits() | Self::Sinks.bits() | Self::Sources.bits() | Self::Fields.bits();
    }
}

impl ResolvableIds {
    fn includes_fields(self) -> bool {
        self.contains(Self::Fields)
    }

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

/// Used to translate IDs into concrete types for a given analysis run
#[derive(Serialize)]
pub struct ReportIdMap {
    pub fields: Option<HashMap<FieldId, FieldSpecAndDisplay>>,
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
    pub fn new_empty() -> Self {
        Self {
            fields: None,
            methods: None,
            sinks: None,
            sources: None,
        }
    }
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

    pub fn get_field(&self, id: FieldId) -> Result<&FieldSpecAndDisplay, IdLookupError> {
        Self::get(&self.fields, id, ResolvableIds::Fields)
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
struct CreateTaintAnalysisSetup {
    graph: GraphSqliteAttach,
}

impl CustomizeConnection<SqliteConnection, r2d2::Error> for CreateTaintAnalysisSetup {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), r2d2::Error> {
        apply_connection_pragmas(conn)?;

        self.graph.attach(conn)?;

        // Must be applied after the graph is attached which is why this is in here instead of in
        // the SQL migration.

        let sinks_triggers = r#"CREATE TEMP TRIGGER update_sink_filters_method
AFTER INSERT ON nodes
WHEN new.kind = 'call'
BEGIN
    INSERT INTO sink_filters (node, kind, class, name, args, ret)
    SELECT
        new.id,
        new.kind,
        classes.name,
        methods.name,
        methods.args,
        methods.ret
    FROM methods
    JOIN classes ON classes.id = methods.class
    WHERE methods.id = new.graph_id;
END;

CREATE TEMP TRIGGER update_sink_filters_fields
AFTER INSERT ON nodes
WHEN new.kind = 'field'
BEGIN
    INSERT INTO sink_filters (node, kind, class, name)
    SELECT
        new.id,
        new.kind,
        classes.name,
        class_fields.name
    FROM class_fields
    JOIN classes ON classes.id = class_fields.class
    WHERE class_fields.id = new.graph_id;
END;
"#;

        _ = query!(sql_query(sinks_triggers)).execute(conn)?;

        Ok(())
    }
}

/// A database connection used for building a taint analysis DB
///
/// Use a [GraphTaintAnalysisDb] or [TaintAnalysisDb] for querying a taint analysis database, this
/// is just for building them.
pub struct TaintAnalysisDbWriter {
    db: TaintAnalysisDb,
    graph: GraphSqliteDatabase,
}

impl Deref for TaintAnalysisDbWriter {
    type Target = TaintAnalysisDb;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl GraphTaintAnalysisDb {
    /// Consume this into a [TaintAnalysisDbWriter]
    pub fn into_writer(self) -> TaintAnalysisDbWriter {
        TaintAnalysisDbWriter {
            db: self.db,
            graph: self.graph,
        }
    }
}

impl TaintAnalysisDbWriter {
    /// Consume this writer into a [GraphTaintAnalysisDb]
    pub fn into_graph(self) -> GraphTaintAnalysisDb {
        GraphTaintAnalysisDb {
            db: self.db,
            graph: self.graph,
        }
    }

    pub fn graph(&self) -> &GraphSqliteDatabase {
        &self.graph
    }

    pub fn new_from_path<S: AsRef<str> + ?Sized>(ctx: &dyn Context, path: &S) -> db::Result<Self> {
        let graph_path = ctx.get_sqlite_dir()?.join(GRAPH_DATABASE_FILE_NAME);
        let graph_path_str = path_must_str(&graph_path);

        let db = Db::new_from_path_with(
            path,
            MIGRATIONS,
            #[cfg(test)]
            MIGRATIONS,
            Some(Box::new(CreateTaintAnalysisSetup {
                graph: GraphSqliteAttach {
                    path: graph_path_str.into(),
                },
            })),
        )?;

        if db.check_fts5() {
            Self::setup_fts5(&db)?;
        }

        let taint_db = TaintAnalysisDb { db: db.clone() };
        let graph = GraphSqliteDatabase::wrap(db);

        Ok(Self {
            db: taint_db,
            graph,
        })
    }

    fn setup_fts5(db: &Db) -> db::Result<()> {
        let setup_sql = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS sink_filters_fts USING fts5(
    class,
    name,
    args,
    ret,
    content='sink_filters',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS sink_filters_insert_fts5
AFTER INSERT ON sink_filters
BEGIN
    INSERT INTO sink_filters_fts(rowid, class, name, args, ret)
    VALUES (new.id, new.class, new.name, new.args, new.ret);
END;

CREATE TRIGGER IF NOT EXISTS sink_filters_delete_fts5 
AFTER DELETE ON sink_filters
BEGIN
    INSERT INTO sink_filters_fts(sink_filters_fts, rowid, class, name, args, ret)
    VALUES ('delete', old.id, old.class, old.name, old.args, old.ret);
END;


CREATE TRIGGER IF NOT EXISTS sink_filters_update_fts5
AFTER UPDATE ON sink_filters
BEGIN
    INSERT INTO sink_filters_fts(sink_filters_fts, rowid, class, name, args, ret)
    VALUES ('delete', old.id, old.class, old.name, old.args, old.ret);
    INSERT INTO sink_filters_fts(rowid, class, name, args, ret)
    VALUES (new.id, new.class, new.name, new.args, new.ret);
END;

INSERT INTO sink_filters_fts(rowid, class, name, args, ret)
SELECT id, class, name, args, ret
FROM sink_filters;
 "#;

        db.write(|c| {
            _ = query!(sql_query(setup_sql)).execute(c)?;
            Ok(())
        })
    }
}

#[derive(Debug)]
struct GraphSqliteAttach {
    path: String,
}

impl GraphSqliteAttach {
    fn attach(&self, conn: &mut SqliteConnection) -> Result<(), r2d2::Error> {
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

impl CustomizeConnection<SqliteConnection, r2d2::Error> for GraphSqliteAttach {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), r2d2::Error> {
        apply_connection_pragmas(conn)?;
        self.attach(conn)
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
diesel::allow_tables_to_appear_in_same_query!(nodes, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(taint_sources, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(graphs, sink_filters_fts);
diesel::allow_tables_to_appear_in_same_query!(analyzed_methods, sink_filters_fts);

diesel::joinable!(
    analyzed_methods -> methods (method)
);
macro_rules! can_methodspec {
    ($table:ident) => {
        diesel::allow_tables_to_appear_in_same_query!($table, methods);
        diesel::allow_tables_to_appear_in_same_query!($table, sources);
        diesel::allow_tables_to_appear_in_same_query!($table, classes);
        diesel::allow_tables_to_appear_in_same_query!($table, class_fields);
    };
}

can_methodspec!(analyzed_methods);
can_methodspec!(nodes);
can_methodspec!(taint_sources);
can_methodspec!(graphs);
can_methodspec!(edges);
can_methodspec!(sink_filters);

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

/// A [FieldSpec] with the smali form attached for display
#[derive(Clone, Debug, Serialize)]
pub struct FieldSpecAndDisplay {
    pub spec: FieldSpec,
    pub display: String,
}

impl Deref for FieldSpecAndDisplay {
    type Target = FieldSpec;
    fn deref(&self) -> &Self::Target {
        &self.spec
    }
}

impl fmt::Display for FieldSpecAndDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display)
    }
}

impl AsRef<str> for FieldSpecAndDisplay {
    fn as_ref(&self) -> &str {
        &self.display
    }
}

pub struct AnalyzedMethodSpec {
    pub analysis_id: AnalyzedMethodId,
    /// False when the call graph led here rather than the method being asked for directly
    pub direct: bool,
    pub ngraphs: usize,
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
pub enum UnresolvedTaintSink {
    Phi,
    Array,
    Call(MethodId),
    Field(FieldId),
    ExternalCall(ExternalCallSinkId),
    ExternalField(ExternalFieldSinkId),
    Instruction(InstructionSinkId),
}

#[derive(Serialize, Debug)]
pub enum ResolvedTaintSink<'a> {
    Phi,
    Array,
    Call(&'a MethodSpecAndDisplay),
    Field(&'a FieldSpecAndDisplay),
    Instruction(&'a str),
    ExternalCall(&'a str),
    ExternalField(&'a str),
}

impl<'a> AsRef<str> for ResolvedTaintSink<'a> {
    fn as_ref(&self) -> &str {
        match self {
            Self::Phi => "<Phi(...)>",
            Self::Array => "<Array>",
            Self::Call(it) => &it.display,
            Self::Field(it) => &it.display,
            Self::Instruction(it) | Self::ExternalCall(it) | Self::ExternalField(it) => it,
        }
    }
}

impl<'a> fmt::Display for ResolvedTaintSink<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl<'a> ResolvedTaintSink<'a> {
    fn resolve(
        unresolved: UnresolvedTaintSink,
        map: &'a ReportIdMap,
    ) -> Result<Self, ResolveError> {
        type USink = UnresolvedTaintSink;

        Ok(match unresolved {
            USink::Array => Self::Array,
            USink::Phi => Self::Phi,
            USink::Call(id) => {
                let method = map.get_method(id)?;
                Self::Call(method)
            }
            USink::Field(id) => {
                let field = map.get_field(id)?;
                Self::Field(field)
            }
            USink::Instruction(id) => {
                let sink = map.get_sink(id)?;
                Self::Instruction(&sink.display)
            }
            USink::ExternalCall(id) => {
                let sink = map.get_sink(id)?;
                Self::ExternalCall(&sink.display)
            }

            USink::ExternalField(id) => {
                let sink = map.get_sink(id)?;
                Self::ExternalField(&sink.display)
            }
        })
    }
}

#[derive(Serialize, Debug)]
pub struct ResolvedNode<'a> {
    pub id: NodeId,
    pub sink: ResolvedTaintSink<'a>,
    pub in_method: &'a MethodSpecAndDisplay,
    /// See [UnresolvedNode::parent]
    pub parent: Option<NodeId>,
    pub depth: u32,
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
    FTS5(String),
    ClassContains(String),
    MethodContains(String),
    FieldContains(String),
    RetContains(String),
    SigContains(String),
    MinLength(usize),
    MaxLength(usize),
}

impl Filter {
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

            "fts5" => {
                let m = value.ok_or_else(|| anyhow::Error::msg("fts5 needs a value"))?;
                Self::FTS5(m.into())
            }

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
        ResolvedTaintSink::resolve(*self, map)
    }
}

/// A collection of all graphs reachable via some taint source
pub struct UnresolvedTaintGraphs {
    pub graphs: Vec<UnresolvedTaintGraph>,
    /// If this is [UnresolvedOrigin::CallGraph], it is the chains until the start method. This
    /// happens when the taint analysis was run with seeding. Note that this is multi dimensional
    /// because there can be multiple paths to the analyzed method. This list is complete as far as
    /// the graph database is concerned.
    pub origin: UnresolvedOrigin,
}

impl Deref for UnresolvedTaintGraphs {
    type Target = Vec<UnresolvedTaintGraph>;
    fn deref(&self) -> &Self::Target {
        &self.graphs
    }
}

/// One node reached by tainted data, completely unresolved
pub struct UnresolvedNode {
    pub id: NodeId,
    pub location: MethodId,
    pub sink: UnresolvedTaintSink,
    /// The node this one was first discovered from, `None` for the entry node.
    ///
    /// This is a convenience for laying the graph out as a tree. It is one incoming edge out of
    /// however many there are: [UnresolvedTaintGraph::edges] is the authoritative structure.
    pub parent: Option<NodeId>,
    /// Distance from the entry node along the discovery path, see [Self::parent]
    pub depth: u32,
}

impl UnresolvedNode {
    fn resolve<'a>(&self, map: &'a ReportIdMap) -> Result<ResolvedNode<'a>, ResolveError> {
        Ok(ResolvedNode {
            id: self.id,
            sink: self.sink.resolve(map)?,
            in_method: map.get_method(self.location)?,
            parent: self.parent,
            depth: self.depth,
        })
    }
}

/// One taint graph in a report completely unresolved; use a [ReportIdMap] to resolve as needed or
/// use [Self::resolve] to resolve it.
///
/// The graph is given as its complete node and edge sets rather than as a tree. A tree can only
/// hold one incoming edge per node, and roughly a tenth of these nodes are reached more than one
/// way, which is usually the interesting part: data arriving from `getData` is not the same
/// finding as the same data arriving from `getStringExtra`.
pub struct UnresolvedTaintGraph {
    pub id: SubgraphId,
    pub source: TaintSourceId,
    pub entry: NodeId,
    pub nodes: Vec<UnresolvedNode>,
    /// Every edge in the graph, including the ones no tree could express
    pub edges: Vec<(NodeId, NodeId)>,
}

impl UnresolvedTaintGraph {
    /// How many of this graph's nodes are [UnresolvedTaintSink::Phi]
    pub fn phis(&self) -> usize {
        self.nodes
            .iter()
            .filter(|it| matches!(it.sink, UnresolvedTaintSink::Phi))
            .count()
    }
}

#[derive(Serialize, Debug)]
pub struct ResolvedTaintGraph<'a> {
    pub id: SubgraphId,
    pub source: &'a TaintSourceAndDisplay,
    pub entry: NodeId,
    pub nodes: Vec<ResolvedNode<'a>>,
    /// Every edge in the graph, see [UnresolvedTaintGraph::edges]
    pub edges: Vec<(NodeId, NodeId)>,
}

impl<'a> ResolvedTaintGraph<'a> {
    /// How many of this graph's nodes are [ResolvedTaintSink::Phi]
    pub fn phis(&self) -> usize {
        self.nodes
            .iter()
            .filter(|it| matches!(it.sink, ResolvedTaintSink::Phi))
            .count()
    }

    fn resolve(
        unresolved: &UnresolvedTaintGraph,
        map: &'a ReportIdMap,
    ) -> Result<Self, ResolveError> {
        let mut nodes = Vec::with_capacity(unresolved.nodes.len());
        for node in &unresolved.nodes {
            nodes.push(node.resolve(map)?);
        }

        Ok(ResolvedTaintGraph {
            id: unresolved.id,
            source: map.get_source(unresolved.source)?,
            entry: unresolved.entry,
            nodes,
            edges: unresolved.edges.clone(),
        })
    }
}

impl UnresolvedTaintGraph {
    pub fn resolve<'a>(
        &self,
        map: &'a ReportIdMap,
    ) -> Result<ResolvedTaintGraph<'a>, ResolveError> {
        ResolvedTaintGraph::resolve(self, map)
    }
}

#[derive(Yokeable)]
pub struct ResolvedTaintGraphs<'a> {
    pub graphs: Vec<ResolvedTaintGraph<'a>>,
    pub origin: ResolvedOrigin,
}

impl<'a> Deref for ResolvedTaintGraphs<'a> {
    type Target = Vec<ResolvedTaintGraph<'a>>;
    fn deref(&self) -> &Self::Target {
        &self.graphs
    }
}

pub struct YokedResolvedTaintGraphs(yoke::Yoke<ResolvedTaintGraphs<'static>, Rc<ReportIdMap>>);

impl Container for YokedResolvedTaintGraphs {
    type Item<'a> = ResolvedTaintGraph<'a>;
    fn len(&self) -> usize {
        self.0.get().len()
    }
    fn get_item<'a>(&'a self, index: usize) -> &'a Self::Item<'a> {
        &self.0.get().graphs[index]
    }
}

pub struct ResolvedTaintGraphsIter<'a> {
    graphs: &'a [ResolvedTaintGraph<'a>],
    at: usize,
}

impl<'a> Iterator for ResolvedTaintGraphsIter<'a> {
    type Item = &'a ResolvedTaintGraph<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let graph = self.graphs.get(self.at)?;
        self.at += 1;
        Some(graph)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.graphs.len() - self.at;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for ResolvedTaintGraphsIter<'a> {}

impl YokedResolvedTaintGraphs {
    pub fn iter(&self) -> ResolvedTaintGraphsIter<'_> {
        let graphs = &self.0.get().graphs;
        ResolvedTaintGraphsIter { graphs, at: 0 }
    }

    pub fn get<'a>(&'a self) -> &'a ResolvedTaintGraphs<'a> {
        self.0.get()
    }
}

impl ResolvedTaintGraphs<'_> {
    pub fn resolve<'a>(
        graphs: UnresolvedTaintGraphs,
        map: &'a ReportIdMap,
    ) -> Result<ResolvedTaintGraphs<'a>, ResolveError> {
        let mut resolved = Vec::new();
        for graph in graphs.graphs {
            resolved.push(ResolvedTaintGraph::resolve(&graph, map)?);
        }

        let origin = match graphs.origin {
            UnresolvedOrigin::Direct => ResolvedOrigin::Direct,
            UnresolvedOrigin::Indirect => ResolvedOrigin::Indirect,
        };

        Ok(ResolvedTaintGraphs {
            graphs: resolved,
            origin,
        })
    }

    /// Resolve into a yoked [ResolvedTaintGraph] with the [ReportIdMap] attached as the cart. This
    /// takes ownership of the map, but the map can still be used via the cart.
    pub fn resolve_yoked(
        graphs: UnresolvedTaintGraphs,
        map: ReportIdMap,
    ) -> Result<YokedResolvedTaintGraphs, ResolveError> {
        let rc_map = Rc::new(map);

        yoke::Yoke::try_attach_to_cart(rc_map, |map| Self::resolve(graphs, map))
            .map(YokedResolvedTaintGraphs)
    }
}

/// One row produced by [GraphTaintAnalysisDb::nodes_for_analysis]: a node reachable from some
/// graph's entry node, with just enough information to tell what it is
#[derive(QueryableByName, Debug)]
#[diesel(check_for_backend(Sqlite))]
struct WalkedNode {
    #[diesel(sql_type = Integer)]
    location: i32,
    #[diesel(sql_type = Text)]
    kind: SinkKind,
    #[diesel(sql_type = Nullable<Integer>)]
    sink_id: Option<i32>,
    /// Either `graph.methods.id` or `graph.class_fields.id`, depending on [Self::kind]
    #[diesel(sql_type = Nullable<Integer>)]
    graph_ref: Option<i32>,
}

/// One row of a graph's node tree, as walked from its entry node down through [edges]
///
/// `parent_id` is `None` only for the entry node of `graph_id`.
#[derive(QueryableByName, Debug)]
#[diesel(check_for_backend(Sqlite))]
struct TreeNodeRow {
    #[diesel(sql_type = Integer)]
    graph_id: i32,
    #[diesel(sql_type = Nullable<Integer>)]
    parent_id: Option<i32>,
    #[diesel(sql_type = Integer)]
    node_id: i32,
    #[diesel(sql_type = Integer)]
    depth: i32,
    #[diesel(sql_type = Integer)]
    location: i32,
    #[diesel(sql_type = Text)]
    kind: SinkKind,
    #[diesel(sql_type = Nullable<Integer>)]
    sink_id: Option<i32>,
    /// Either `graph.methods.id` or `graph.class_fields.id`, depending on [Self::kind]
    #[diesel(sql_type = Nullable<Integer>)]
    graph_ref: Option<i32>,
}

/// One edge between two nodes of the same graph
#[derive(QueryableByName, Debug)]
#[diesel(check_for_backend(Sqlite))]
struct GraphEdgeRow {
    #[diesel(sql_type = Integer)]
    graph_id: i32,
    #[diesel(sql_type = Integer)]
    src: i32,
    #[diesel(sql_type = Integer)]
    dst: i32,
}

impl TreeNodeRow {
    /// The [UnresolvedTaintSink] this row's `kind` names
    fn sink(&self) -> anyhow::Result<UnresolvedTaintSink> {
        Ok(match self.kind {
            SinkKind::Phi => UnresolvedTaintSink::Phi,
            SinkKind::Array => UnresolvedTaintSink::Array,
            SinkKind::Call => match self.graph_ref {
                None => bail!("BUG! graph_id NULL for call kind!"),
                Some(v) => UnresolvedTaintSink::Call(v.into()),
            },
            SinkKind::Field => match self.graph_ref {
                None => bail!("BUG! graph_id NULL for field kind!"),
                Some(v) => UnresolvedTaintSink::Field(v.into()),
            },
            SinkKind::Instruction => match self.sink_id {
                None => bail!("BUG! sink_id NULL for instr kind!"),
                Some(v) => UnresolvedTaintSink::Instruction(v.into()),
            },
            SinkKind::ExternalCall => match self.sink_id {
                None => bail!("BUG! sink_id NULL for ext-call kind!"),
                Some(v) => UnresolvedTaintSink::ExternalCall(v.into()),
            },
            SinkKind::ExternalField => match self.sink_id {
                None => bail!("BUG! sink_id NULL for ext-field kind!"),
                Some(v) => UnresolvedTaintSink::ExternalField(v.into()),
            },
        })
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

    fn quote_fts5(s: &str) -> String {
        format!("\"{}\"", s.replace('"', "\"\""))
    }

    /// Collect the text-search and shape filters shared by the matching queries below
    ///
    /// Returns the FTS5 match expressions (already formatted per-column) plus the no-phi/length
    /// filters. Bails if a text filter is used without FTS5 support.
    fn collect_filters(
        filters: &[Filter],
        text_search_allowed: bool,
    ) -> anyhow::Result<(Vec<String>, bool, Option<usize>, Option<usize>)> {
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
                Filter::NoPhi => no_phi = true,
                Filter::MinLength(len) => min_length = Some(*len),
                Filter::MaxLength(len) => max_length = Some(*len),
                _ if !text_search_allowed => bail!("text search requires FTS5"),
                Filter::FTS5(s) => fts5_queries.push(Self::quote_fts5(s)),
                Filter::SigContains(s) => {
                    fts5_queries.push(format!("args:{}", Self::quote_fts5(s)))
                }
                Filter::ClassContains(s) => {
                    fts5_queries.push(format!("class:{}", Self::quote_fts5(s)))
                }
                Filter::MethodContains(s) | Filter::FieldContains(s) => {
                    fts5_queries.push(format!("name:{}", Self::quote_fts5(s)))
                }
                Filter::RetContains(s) => fts5_queries.push(format!("ret:{}", Self::quote_fts5(s))),
            }
        }

        Ok((fts5_queries, no_phi, min_length, max_length))
    }

    fn get_analysis_matching_fts5(
        &self,
        filters: &[Filter],
        filter_fields: bool,
    ) -> anyhow::Result<Vec<AnalyzedMethodId>> {
        let (fts5_queries, no_phi, min_length, max_length) = Self::collect_filters(filters, true)?;

        let kind_condition = if filter_fields {
            "sink_filters.kind = 'field'"
        } else {
            "sink_filters.kind != 'field'"
        };

        // Only reach for graph_metadata when a filter actually reads it. It is written per
        // completed analysis, so a graph without a row would otherwise be dropped by the join
        // even when the text match found it.
        let metadata_join = if no_phi || min_length.is_some() || max_length.is_some() {
            "INNER JOIN graph_metadata ON graph_metadata.graph = t.graph"
        } else {
            ""
        };

        let fts_condition = if fts5_queries.is_empty() {
            ""
        } else {
            "AND sink_filters_fts MATCH ?"
        };

        let no_phi_condition = if no_phi {
            "AND graph_metadata.nphi = 0"
        } else {
            ""
        };
        let min_length_condition = if min_length.is_some() {
            "AND graph_metadata.depth >= ?"
        } else {
            ""
        };
        let max_length_condition = if max_length.is_some() {
            "AND graph_metadata.depth <= ?"
        } else {
            ""
        };

        let sql = format!(
            r#"SELECT DISTINCT graphs.analyzed_method AS id
        FROM reachable_nodes AS t
        INNER JOIN graphs
            ON graphs.id = t.graph
        {metadata_join}
        INNER JOIN sink_filters
            ON sink_filters.node = t.node
        INNER JOIN sink_filters_fts
            ON sink_filters_fts.rowid = sink_filters.id
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
        let (_, no_phi, min_length, max_length) = Self::collect_filters(filters, false)?;

        let no_phi_condition = if no_phi {
            "AND EXISTS (SELECT 1 FROM graph_metadata AS gm WHERE gm.analyzed_method = am.id AND gm.nphi = 0)"
        } else {
            ""
        };
        let min_length_condition = if min_length.is_some() {
            "AND EXISTS (SELECT 1 FROM graph_metadata AS gm WHERE gm.analyzed_method = am.id AND gm.depth >= ?)"
        } else {
            ""
        };
        let max_length_condition = if max_length.is_some() {
            "AND EXISTS (SELECT 1 FROM graph_metadata AS gm WHERE gm.analyzed_method = am.id AND gm.depth <= ?)"
        } else {
            ""
        };

        let sql = format!(
            r#"SELECT am.id AS id
        FROM analyzed_methods am
        WHERE true
        {no_phi_condition}
        {min_length_condition}
        {max_length_condition}
        "#
        );

        let mut query = sql_query(sql).into_boxed();

        if let Some(len) = min_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        if let Some(len) = max_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        Ok(self.query(|c| query!(query).get_results::<AnalyzedMethodId>(c))?)
    }

    pub fn get_graphs_matching(
        &self,
        analysis_id: AnalyzedMethodId,
        filters: &[Filter],
    ) -> anyhow::Result<Vec<SubgraphId>> {
        if filters.len() == 0 {
            return Ok(self.query(|c| {
                query!(graphs::table
                    .select(graphs::id)
                    .filter(graphs::analyzed_method.eq(analysis_id)))
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
            return self.get_graphs_matching_fts5(analysis_id, filters, filter_fields);
        }
        self.get_graphs_matching_no_fts5(analysis_id, filters)
    }

    fn get_graphs_matching_fts5(
        &self,
        analysis_id: AnalyzedMethodId,
        filters: &[Filter],
        filter_fields: bool,
    ) -> anyhow::Result<Vec<SubgraphId>> {
        let (fts5_queries, no_phi, min_length, max_length) = Self::collect_filters(filters, true)?;

        let kind_condition = if filter_fields {
            "sink_filters.kind = 'field'"
        } else {
            "sink_filters.kind != 'field'"
        };

        // Only reach for graph_metadata when a filter actually reads it. It is written per
        // completed analysis, so a graph without a row would otherwise be dropped by the join
        // even when the text match found it.
        let metadata_join = if no_phi || min_length.is_some() || max_length.is_some() {
            "INNER JOIN graph_metadata ON graph_metadata.graph = t.graph"
        } else {
            ""
        };

        let fts_condition = if fts5_queries.is_empty() {
            ""
        } else {
            "AND sink_filters_fts MATCH ?"
        };

        let no_phi_condition = if no_phi {
            "AND graph_metadata.nphi = 0"
        } else {
            ""
        };
        let min_length_condition = if min_length.is_some() {
            "AND graph_metadata.depth >= ?"
        } else {
            ""
        };
        let max_length_condition = if max_length.is_some() {
            "AND graph_metadata.depth <= ?"
        } else {
            ""
        };

        let sql = format!(
            r#"SELECT DISTINCT t.graph AS id
        FROM reachable_nodes AS t
        INNER JOIN graphs
            ON graphs.id = t.graph
        {metadata_join}
        INNER JOIN sink_filters
            ON sink_filters.node = t.node
        INNER JOIN sink_filters_fts
            ON sink_filters_fts.rowid = sink_filters.id
        WHERE graphs.analyzed_method = ?
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

        Ok(self.query(|conn| query!(query).get_results::<SubgraphId>(conn))?)
    }

    fn get_graphs_matching_no_fts5(
        &self,
        analysis_id: AnalyzedMethodId,
        filters: &[Filter],
    ) -> anyhow::Result<Vec<SubgraphId>> {
        let (_, no_phi, min_length, max_length) = Self::collect_filters(filters, false)?;

        let no_phi_condition = if no_phi { "AND gm.nphi = 0" } else { "" };
        let min_length_condition = if min_length.is_some() {
            "AND gm.depth >= ?"
        } else {
            ""
        };
        let max_length_condition = if max_length.is_some() {
            "AND gm.depth <= ?"
        } else {
            ""
        };

        let sql = format!(
            r#"SELECT gm.graph AS id
        FROM graph_metadata AS gm
        WHERE gm.analyzed_method = ?
        {no_phi_condition}
        {min_length_condition}
        {max_length_condition}
        "#
        );

        let mut query = sql_query(sql).into_boxed();

        query = query.bind::<Integer, _>(analysis_id.raw());

        if let Some(len) = min_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        if let Some(len) = max_length {
            query = query.bind::<Integer, _>(len as i32);
        }

        Ok(self.query(|c| query!(query).get_results::<SubgraphId>(c))?)
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

    /// Every node reached by tainted data for `id`, or every node in the database if `id` is
    /// `None`
    ///
    /// A node has no column naming its owning graph, so membership comes from [reachable_nodes],
    /// which records every node reachable from each graph's entry node.
    fn nodes_for_analysis(&self, id: Option<AnalyzedMethodId>) -> db::Result<Vec<WalkedNode>> {
        let root_filter = if id.is_some() {
            "WHERE g.analyzed_method = ?"
        } else {
            ""
        };

        let sql = format!(
            r#"
        SELECT DISTINCT rn.location AS location, n.kind AS kind, n.sink_id AS sink_id,
               n.graph_id AS graph_ref
        FROM reachable_nodes AS rn
        INNER JOIN graphs g ON g.id = rn.graph
        INNER JOIN nodes n ON n.id = rn.node
        {root_filter}
        "#
        );

        let mut query = sql_query(sql).into_boxed();
        if let Some(id) = id {
            query = query.bind::<Integer, _>(id.raw());
        }

        self.query(|c| query!(query).get_results::<WalkedNode>(c))
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
                        .inner_join(
                            taint_sources::table.on(taint_sources::source_id.eq(source_params::id))
                        )
                        .inner_join(graphs::table.on(graphs::source.eq(taint_sources::id)))
                        .filter(taint_sources::kind.eq(SourceKind::Param))
                        .filter(OptionalId::new(graphs::analyzed_method, id))
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
                        .inner_join(
                            taint_sources::table.on(taint_sources::source_id.eq(source_fields::id))
                        )
                        .inner_join(graphs::table.on(graphs::source.eq(taint_sources::id)))
                        .filter(taint_sources::kind.eq(SourceKind::Field))
                        .filter(OptionalId::new(graphs::analyzed_method, id))
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
                        .inner_join(
                            taint_sources::table.on(taint_sources::source_id.eq(source_calls::id))
                        )
                        .inner_join(graphs::table.on(graphs::source.eq(taint_sources::id)))
                        .filter(taint_sources::kind.eq(SourceKind::Call))
                        .filter(OptionalId::new(graphs::analyzed_method, id))
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
        let rows = self.nodes_for_analysis(id)?;

        let mut ext_call_ids = HashSet::new();
        let mut ext_field_ids = HashSet::new();
        let mut instruction_ids = HashSet::new();

        for row in &rows {
            let Some(sink_id) = row.sink_id else {
                continue;
            };
            match row.kind {
                SinkKind::ExternalCall => _ = ext_call_ids.insert(sink_id),
                SinkKind::ExternalField => _ = ext_field_ids.insert(sink_id),
                SinkKind::Instruction => _ = instruction_ids.insert(sink_id),
                SinkKind::Call | SinkKind::Field | SinkKind::Phi | SinkKind::Array => {}
            }
        }

        let mut map = HashMap::new();

        if !ext_call_ids.is_empty() {
            let ids = Vec::from_iter(ext_call_ids);
            map.extend(
                self.query(|c| {
                    query!(external_call_sinks::table
                        .filter(external_call_sinks::id.eq_any(&ids))
                        .select((
                            external_call_sinks::id,
                            external_call_sinks::class,
                            external_call_sinks::name,
                            external_call_sinks::signature,
                        )))
                    .get_results::<(ExternalCallSinkId, ClassName, String, String)>(c)
                })?
                .into_iter()
                .map(|(id, class, name, signature)| {
                    let display = format!("{class}->{name}({signature})");
                    (
                        SinkId::ExternalCall(id),
                        SinkDefAndDisplay {
                            sink_def: SinkDef::ExternalCall {
                                class,
                                name,
                                signature,
                            },
                            display,
                        },
                    )
                }),
            );
        }

        if !ext_field_ids.is_empty() {
            let ids = Vec::from_iter(ext_field_ids);
            map.extend(
                self.query(|c| {
                    query!(external_field_sinks::table
                        .filter(external_field_sinks::id.eq_any(&ids))
                        .select((
                            external_field_sinks::id,
                            external_field_sinks::class,
                            external_field_sinks::name,
                        )))
                    .get_results::<(ExternalFieldSinkId, ClassName, String)>(c)
                })?
                .into_iter()
                .map(|(id, class, name)| {
                    let display = format!("{class}->{name}");
                    (
                        SinkId::ExternalField(id),
                        SinkDefAndDisplay {
                            sink_def: SinkDef::ExternalField { class, name },
                            display,
                        },
                    )
                }),
            );
        }

        if !instruction_ids.is_empty() {
            let ids = Vec::from_iter(instruction_ids);
            map.extend(
                self.query(|c| {
                    query!(instruction_sinks::table
                        .filter(instruction_sinks::id.eq_any(&ids))
                        .select((instruction_sinks::id, instruction_sinks::instruction)))
                    .get_results::<(InstructionSinkId, String)>(c)
                })?
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
        }

        Ok(map)
    }

    fn get_methods_map(
        &self,
        ana: Option<&AnalyzedMethod>,
    ) -> db::Result<HashMap<MethodId, MethodSpecAndDisplay>> {
        let id = ana.map(|it| it.id);

        // Grab every method referenced by this analyzed method. This includes:
        //  - reachable_nodes.location
        //  - nodes.graph_id when the node's kind is 'call'
        //  - analyzed_methods.method

        let mut method_ids = HashSet::new();

        for row in self.nodes_for_analysis(id)? {
            method_ids.insert(row.location);
            if row.kind == SinkKind::Call {
                if let Some(method) = row.graph_ref {
                    method_ids.insert(method);
                }
            }
        }

        method_ids.extend(
            self.query(|c| {
                query!(analyzed_methods::table
                    .select(analyzed_methods::method)
                    .filter(OptionalId::new(analyzed_methods::id, id))
                    .distinct())
                .get_results::<MethodId>(c)
            })?
            .into_iter()
            .map(DatabaseId::id),
        );

        if method_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let ids = Vec::from_iter(method_ids);

        let rows = self.query(|c| {
            query!(methods::table
                .inner_join(classes::table)
                .inner_join(sources::table.on(sources::id.eq(classes::source)))
                .select(MethodSpecRow::as_select())
                .filter(methods::id.eq_any(&ids)))
            .get_results::<MethodSpecRow>(c)
        })?;

        Ok(HashMap::from_iter(rows.into_iter().map(|it| {
            let spec = MethodSpec::from(it);
            let display = spec.as_smali();
            (spec.id, MethodSpecAndDisplay { spec, display })
        })))
    }

    fn get_fields_map(
        &self,
        ana: Option<&AnalyzedMethod>,
    ) -> db::Result<HashMap<FieldId, FieldSpecAndDisplay>> {
        let id = ana.map(|it| it.id);

        let field_ids: HashSet<i32> = self
            .nodes_for_analysis(id)?
            .into_iter()
            .filter(|row| row.kind == SinkKind::Field)
            .filter_map(|row| row.graph_ref)
            .collect();

        if field_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let ids = Vec::from_iter(field_ids);

        let rows = self.query(|c| {
            query!(class_fields::table
                .inner_join(classes::table)
                .inner_join(sources::table.on(sources::id.eq(classes::source)))
                .select(FieldSpecRow::as_select())
                .filter(class_fields::id.eq_any(&ids)))
            .get_results::<FieldSpecRow>(c)
        })?;

        Ok(HashMap::from_iter(rows.into_iter().map(|it| {
            let spec = FieldSpec::from(it);
            let display = spec.to_string();
            (spec.id, FieldSpecAndDisplay { spec, display })
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

        let fields = if select.includes_fields() {
            self.get_fields_map(ana).map(Some)?
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
            fields,
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

    /// Turn one graph's rows into its node set
    fn build_nodes(rows: &[TreeNodeRow]) -> anyhow::Result<Vec<UnresolvedNode>> {
        rows.iter()
            .map(|row| {
                Ok(UnresolvedNode {
                    id: row.node_id.into(),
                    location: row.location.into(),
                    sink: row.sink()?,
                    parent: row.parent_id.map(NodeId::from),
                    depth: row.depth as u32,
                })
            })
            .collect()
    }

    /// Get all [UnresolvedTaintGraph]s for the given analysis
    ///
    /// To resolve all of the IDs, use a [ReportIdMap] with [ResolvedTaintGraphs::resolve]
    pub fn get_graphs_for_analysis(
        &self,
        ana: &AnalyzedMethod,
    ) -> anyhow::Result<UnresolvedTaintGraphs> {
        let graph_sources = self.db.query(|c| {
            query!(graphs::table
                .inner_join(
                    analyzed_methods::table.on(analyzed_methods::id.eq(graphs::analyzed_method))
                )
                .select((graphs::id, graphs::source, graphs::entry_node))
                .filter(graphs::analyzed_method.eq(ana.id))
                .filter(analyzed_methods::status.eq(MethodStatus::Done)))
            .get_results::<(SubgraphId, TaintSourceId, NodeId)>(c)
        })?;

        let origin = if ana.direct {
            UnresolvedOrigin::Direct
        } else {
            UnresolvedOrigin::Indirect
        };

        if graph_sources.len() == 0 {
            return Ok(UnresolvedTaintGraphs {
                graphs: vec![],
                origin,
            });
        }

        let sql = r#"
        SELECT rn.graph AS graph_id,
               rn.parent AS parent_id,
               rn.node AS node_id,
               rn.depth AS depth,
               rn.location AS location,
               n.kind AS kind,
               n.sink_id AS sink_id,
               n.graph_id AS graph_ref
        FROM reachable_nodes AS rn
        INNER JOIN graphs g
            ON g.id = rn.graph
        INNER JOIN nodes n 
            ON n.id = rn.node
        WHERE g.analyzed_method = ?
        ORDER BY rn.graph, rn.depth, rn.node
        "#;

        let query = sql_query(sql).bind::<Integer, _>(ana.id.raw());
        let rows = self
            .db
            .query(|c| query!(query).get_results::<TreeNodeRow>(c))?;

        let mut rows_by_graph: HashMap<i32, Vec<TreeNodeRow>> = HashMap::new();
        for row in rows {
            rows_by_graph.entry(row.graph_id).or_default().push(row);
        }

        // Every edge between two nodes of the same graph
        let edge_sql = r#"
        SELECT rs.graph AS graph_id, e.src AS src, e.dst AS dst
        FROM edges AS e
        INNER JOIN reachable_nodes AS rs
            ON rs.node = e.src
        INNER JOIN reachable_nodes AS rd
            ON rd.node = e.dst AND rd.graph = rs.graph
        INNER JOIN graphs g
            ON g.id = rs.graph
        WHERE g.analyzed_method = ?
        GROUP BY rs.graph, e.src, e.dst
        "#;

        let edge_query = sql_query(edge_sql).bind::<Integer, _>(ana.id.raw());
        let edge_rows = self
            .db
            .query(|c| query!(edge_query).get_results::<GraphEdgeRow>(c))?;

        let mut edges_by_graph: HashMap<i32, Vec<(NodeId, NodeId)>> = HashMap::new();
        for row in edge_rows {
            edges_by_graph
                .entry(row.graph_id)
                .or_default()
                .push((row.src.into(), row.dst.into()));
        }

        let mut graphs = Vec::with_capacity(graph_sources.len());

        for (id, source, entry) in graph_sources {
            // Every done graph is materialised, so this should not happen. Skipping rather than
            // failing keeps one anomaly from hiding every other graph in the analysis.
            let Some(rows) = rows_by_graph.get(&id.raw()) else {
                log::warn!("graph {id} has no materialised nodes, skipping it");
                continue;
            };

            graphs.push(UnresolvedTaintGraph {
                id,
                source,
                entry,
                nodes: Self::build_nodes(rows)?,
                edges: edges_by_graph.remove(&id.raw()).unwrap_or_default(),
            });
        }

        Ok(UnresolvedTaintGraphs { graphs, origin })
    }

    pub fn get_all_analyzed_method_specs(&self) -> db::Result<Vec<AnalyzedMethodSpec>> {
        #[derive(QueryableByName)]
        #[diesel(check_for_backend(Sqlite))]
        pub struct QueryResult {
            #[diesel(sql_type = Integer)]
            analyzed_method: i32,
            #[diesel(sql_type = Bool)]
            direct: bool,
            #[diesel(sql_type = Text)]
            status: MethodStatus,
            #[diesel(sql_type = Nullable<Text>)]
            error: Option<String>,
            #[diesel(sql_type = BigInt)]
            graph_count: i64,
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
SELECT  am.id                     AS analyzed_method,
        am.direct                 AS direct,
        am.status                 AS status,
        am.error                  AS error,
        count(DISTINCT g.id)      AS graph_count,
        c.id                      AS class_id,
        c.name                    AS class,
        m.id                      AS id,
        m.name                    AS name,
        m.args                    AS args,
        m.ret                     AS ret,
        m.access_flags            AS access_flags,
        s.name                    AS source
FROM methods AS m
JOIN analyzed_methods AS am
    ON am.method = m.id
JOIN classes AS c
    ON c.id = m.class
JOIN sources AS s
    ON s.id = c.source
LEFT JOIN graphs AS g
    ON g.analyzed_method = am.id
GROUP BY am.id;"#
            ))
            .get_results::<QueryResult>(c)
        })?;

        Ok(rows
            .into_iter()
            .map(
                |QueryResult {
                     analyzed_method,
                     direct,
                     status,
                     error,
                     graph_count,
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
                    direct,
                    ngraphs: graph_count as usize,
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
                .union(reachable_nodes::table.select(reachable_nodes::location))
                .union(
                    nodes::table
                        .select(nodes::graph_id.assume_not_null())
                        .filter(nodes::graph_id.is_not_null())
                        .filter(nodes::kind.eq(SinkKind::Call)),
                ))
            .get_results::<MethodId>(c)
        })
    }

    pub fn unhide_graph(&self, graph: SubgraphId) -> db::Result<()> {
        self.write(|c| -> db::Result<()> {
            query_exec!(
                delete(_hidden_graphs::table).filter(_hidden_graphs::graph.eq(graph)),
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

    pub fn hide_graph(&self, graph: SubgraphId) -> db::Result<()> {
        self.write(|c| -> db::Result<()> {
            let value = InsertHiddenGraph { graph };
            query_exec!(
                insert_or_ignore_into(_hidden_graphs::table).values(&value),
                c
            )?;
            Ok(())
        })
    }

    pub fn get_hidden_graphs(&self) -> db::Result<Vec<SubgraphId>> {
        Ok(self.query(|c| {
            query!(_hidden_graphs::table.select(_hidden_graphs::graph)).get_results::<SubgraphId>(c)
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
