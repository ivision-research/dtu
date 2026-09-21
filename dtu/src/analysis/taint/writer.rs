use std::collections::HashMap;
use std::hash::Hash;

use diesel::connection::SimpleConnection;
use diesel::dsl::update;
use diesel::prelude::*;
use diesel::sql_types::Integer;
use diesel::{insert_into, insert_or_ignore_into, sql_query, SqliteConnection};
use itertools::Itertools;
use smalisa::instructions::Instruction;
use smalisa::RegisterNumber;

use super::db::{SinkDef, TaintAnalysisDb, TaintAnalysisDbWriter, UnresolvedOrigin};
use super::engine::IdFactories;
use super::models::*;
use super::schema::{
    analyzed_methods, external_call_sinks, external_field_sinks, instruction_sinks, run_info,
    source_calls, source_fields, source_params, taint_sources,
};
use super::TaintSource;
use crate::analysis::taint::schema::{edges, graphs, nodes};
use crate::db::graph::models::{FieldId, MethodId};
use crate::db::graph::GraphDatabase;
use crate::db::{self, query, query_exec, DatabaseId};
use crate::utils::{unix_now, ClassName};
use crate::VERSION;

const INSERT_BUFFER_SIZE: usize = 2048;

macro_rules! insert_or_ignore {
    ($db:expr, $tbl:path, $values:expr) => {{
        $db.write(|c| -> db::Result<()> {
            query_exec!(insert_or_ignore_into($tbl).values($values), c)?;
            Ok(())
        })
    }};
}

macro_rules! insert {
    ($db:expr, $tbl:path, $values:expr) => {{
        $db.write(|c| -> db::Result<()> {
            query_exec!(insert_into($tbl).values($values), c)?;
            Ok(())
        })
    }};
}

pub enum RawSink {
    Phi,
    Array,
    Call(MethodId),
    Field(FieldId),
    Instruction(Instruction),
    ExternalField {
        class: ClassName,
        name: String,
    },
    ExternalCall {
        class: ClassName,
        name: String,
        signature: String,
    },
}

/// Tell the database to do something
///
/// These are passed along a channel from the workers to a dedicated database writer thread. This
/// means that a lot of these IDs are not created by the database but instead inserted into the
/// database. That is the reason you see all these things that make IDs in this file.
pub enum DatabaseAction {
    StartGraph {
        id: SubgraphId,
        entry: NodeId,
        analyzed_method: AnalyzedMethodId,
        source: TaintSourceId,
    },

    /// Create a new node in the database
    AddNode {
        id: NodeId,
        sink: RawSink,
        /// Populated for [RawSink::Call] and [RawSink::ExternalCall] nodes
        registers: Option<Vec<RegisterNumber>>,
    },

    /// Add an edge to the database
    AddEdge {
        src: NodeId,
        dst: NodeId,
        /// The method `dst` sits in
        location: MethodId,
    },

    FinishAnalysis {
        id: AnalyzedMethodId,
    },
}

/// Everything about a run that is known before it starts
pub struct RunMeta {
    options: String,
    schema_version: i32,
    graph_built_at: i64,
    dtu_version: String,
    started_at: i64,
}

impl RunMeta {
    pub fn new(graph: &dyn GraphDatabase, options: String) -> db::Result<Self> {
        Ok(Self {
            options,
            schema_version: TaintAnalysisDb::SCHEMA_VERSION,
            graph_built_at: graph.get_built_at()?,
            dtu_version: VERSION.to_string(),
            started_at: unix_now()
                .map_err(|_| db::Error::Generic("failed to get the unix timestamp".into()))?,
        })
    }

    pub fn as_insert(&self) -> InsertRunInfo<'_> {
        InsertRunInfo {
            options: &self.options,
            schema_version: self.schema_version,
            graph_built_at: self.graph_built_at,
            dtu_version: &self.dtu_version,
            started_at: self.started_at,
            completed: false,
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum BorrowedSinkDef<'a> {
    Instruction(&'a str),
    ExternalCall {
        class: &'a ClassName,
        name: &'a str,
        signature: &'a str,
    },
    ExternalField {
        class: &'a ClassName,
        name: &'a str,
    },
}

impl<'a> From<BorrowedSinkDef<'a>> for SinkDef {
    fn from(value: BorrowedSinkDef<'a>) -> Self {
        match value {
            BorrowedSinkDef::Instruction(ins) => SinkDef::Instruction(String::from(ins)),
            BorrowedSinkDef::ExternalField { class, name } => SinkDef::ExternalField {
                class: class.clone(),
                name: String::from(name),
            },
            BorrowedSinkDef::ExternalCall {
                class,
                name,
                signature,
            } => SinkDef::ExternalCall {
                class: class.clone(),
                name: String::from(name),
                signature: String::from(signature),
            },
        }
    }
}

enum SinkDefWrapper<'a> {
    Borrowed(BorrowedSinkDef<'a>),
    Owned(SinkDef),
}

impl<'a> Hash for SinkDefWrapper<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Borrowed(it) => match it {
                BorrowedSinkDef::Instruction(ins) => ins.hash(state),
                BorrowedSinkDef::ExternalCall {
                    class,
                    name,
                    signature,
                } => {
                    class.hash(state);
                    name.hash(state);
                    signature.hash(state);
                }
                BorrowedSinkDef::ExternalField { class, name } => {
                    class.hash(state);
                    name.hash(state);
                }
            },

            Self::Owned(it) => match it {
                SinkDef::Instruction(ins) => ins.as_str().hash(state),
                SinkDef::ExternalCall {
                    class,
                    name,
                    signature,
                } => {
                    class.hash(state);
                    name.as_str().hash(state);
                    signature.as_str().hash(state);
                }
                SinkDef::ExternalField { class, name } => {
                    class.hash(state);
                    name.as_str().hash(state);
                }
            },
        }
    }
}

impl<'a> PartialEq for SinkDefWrapper<'a> {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::Borrowed(slf) => match other {
                Self::Borrowed(oth) => slf.eq(oth),
                Self::Owned(oth) => oth.equals_borrowed(slf),
            },
            Self::Owned(slf) => match other {
                Self::Owned(oth) => slf.eq(oth),
                Self::Borrowed(oth) => slf.equals_borrowed(oth),
            },
        }
    }
}

impl SinkDef {
    fn equals_borrowed(&self, borrowed: &BorrowedSinkDef<'_>) -> bool {
        match self {
            SinkDef::Instruction(slf) => match borrowed {
                BorrowedSinkDef::Instruction(oth) => slf == oth,
                _ => false,
            },
            SinkDef::ExternalCall {
                class,
                name,
                signature,
            } => match borrowed {
                BorrowedSinkDef::ExternalCall {
                    class: oth_class,
                    name: oth_name,
                    signature: oth_signature,
                } => class == oth_class && name == oth_name && signature == oth_signature,
                _ => false,
            },
            SinkDef::ExternalField { class, name } => match borrowed {
                BorrowedSinkDef::ExternalField {
                    class: oth_class,
                    name: oth_name,
                } => class == oth_class && name == oth_name,
                _ => false,
            },
        }
    }
}

impl<'a> Eq for SinkDefWrapper<'a> {}

impl<'a> From<SinkDef> for SinkDefWrapper<'a> {
    fn from(value: SinkDef) -> Self {
        Self::Owned(value)
    }
}

impl<'a> From<SinkDefWrapper<'a>> for SinkDef {
    fn from(value: SinkDefWrapper<'a>) -> Self {
        match value {
            SinkDefWrapper::Owned(it) => it,
            SinkDefWrapper::Borrowed(it) => it.into(),
        }
    }
}

/// A method the run intends to analyze, known before any analysis happens
pub struct PlannedMethod {
    pub method: MethodId,
    pub origin: UnresolvedOrigin,
}

/// Streams the result of one taint command into an artifact.
pub struct RunWriter<'a> {
    pub(crate) db: &'a TaintAnalysisDbWriter,
    factories: &'a IdFactories,
    next_sink_id: i32,
    ids: HashMap<MethodId, AnalyzedMethodId>,

    // Methods that were completed in a previous run

    // We use 'static because these are never stored as references and i32 because these IDs can
    // reach into multiple tables
    //
    // Note that this is currently unbounded and might get pathological at some point. A better
    // implementation would use a bounded LRU cache and fall back to a database search, I think at
    // least.
    refs: HashMap<SinkDefWrapper<'static>, i32>,
    sources: HashMap<TaintSource, TaintSourceId>,

    edges: Vec<InsertEdge>,
    nodes: Vec<InsertNode>,
    graphs: Vec<InsertGraph>,
}

impl<'a> RunWriter<'a> {
    /// Write a new run info if one doesn't exist and update one that already does
    pub(super) fn initialize(
        db: &'a TaintAnalysisDbWriter,
        info: &InsertRunInfo<'_>,
        factories: &'a IdFactories,
    ) -> db::Result<Self> {
        let have_run_info = db
            .query(|c| {
                run_info::table
                    .select(run_info::started_at)
                    .get_result::<i64>(c)
                    .optional()
            })?
            .is_some();

        if !have_run_info {
            return Self::create_new_run_meta(db, info, factories);
        }

        Self::update_existing_run_meta(db, factories)
    }

    fn update_existing_run_meta(
        db: &'a TaintAnalysisDbWriter,
        factories: &'a IdFactories,
    ) -> db::Result<Self> {
        let started_at = unix_now()
            .map_err(|_| db::Error::Generic("failed to get the unix timestamp".into()))?;

        let (ids, refs, sources) = db.query(
            |c| -> db::Result<(
                HashMap<MethodId, AnalyzedMethodId>,
                HashMap<SinkDefWrapper<'static>, i32>,
                HashMap<TaintSource, TaintSourceId>,
            )> {
                let ids = HashMap::from_iter(
                    query!(analyzed_methods::table
                        .select((analyzed_methods::method, analyzed_methods::id)))
                    .get_results::<(MethodId, AnalyzedMethodId)>(c)?
                    .iter()
                    .copied(),
                );

                let mut refs: HashMap<SinkDefWrapper<'static>, i32> = HashMap::new();

                refs.extend(
                    query!(instruction_sinks::table
                        .select((instruction_sinks::instruction, instruction_sinks::id)))
                    .get_results::<(String, InstructionSinkId)>(c)?
                    .into_iter()
                    .map(|(instruction, id)| (SinkDef::Instruction(instruction).into(), id.id())),
                );

                refs.extend(
                    query!(external_field_sinks::table.select((
                        external_field_sinks::class,
                        external_field_sinks::name,
                        external_field_sinks::id
                    )))
                    .get_results::<(ClassName, String, ExternalFieldSinkId)>(c)?
                    .into_iter()
                    .map(|(class, name, id)| {
                        (SinkDef::ExternalField { class, name }.into(), id.id())
                    }),
                );

                refs.extend(
                    query!(external_call_sinks::table.select((
                        external_call_sinks::class,
                        external_call_sinks::name,
                        external_call_sinks::signature,
                        external_call_sinks::id,
                    )))
                    .get_results::<(ClassName, String, String, ExternalCallSinkId)>(c)?
                    .into_iter()
                    .map(|(class, name, signature, id)| {
                        (
                            SinkDef::ExternalCall {
                                class,
                                name,
                                signature,
                            }
                            .into(),
                            id.id(),
                        )
                    }),
                );

                let mut sources = HashMap::new();

                let params = query!(source_params::table
                    .inner_join(
                        taint_sources::table.on(taint_sources::source_id.eq(source_params::id))
                    )
                    .select((taint_sources::id, source_params::register))
                    .filter(taint_sources::kind.eq(SourceKind::Param)))
                .load::<(TaintSourceId, i32)>(c)?
                .into_iter()
                .map(|(id, reg)| {
                    (
                        TaintSource::Param {
                            register: reg as u16,
                        },
                        id,
                    )
                });

                sources.extend(params);

                let fields = query!(source_fields::table
                    .inner_join(
                        taint_sources::table.on(taint_sources::source_id.eq(source_fields::id))
                    )
                    .select((taint_sources::id, source_fields::class, source_fields::name))
                    .filter(taint_sources::kind.eq(SourceKind::Field)))
                .load::<(TaintSourceId, ClassName, String)>(c)?
                .into_iter()
                .map(|(id, class, name)| (TaintSource::Field { class, name }, id));

                sources.extend(fields);

                let methods = query!(source_calls::table
                    .inner_join(
                        taint_sources::table.on(taint_sources::source_id.eq(source_calls::id))
                    )
                    .select((
                        taint_sources::id,
                        source_calls::class,
                        source_calls::name,
                        source_calls::args,
                        source_calls::ret
                    ))
                    .filter(taint_sources::kind.eq(SourceKind::Call)))
                .load::<(
                    TaintSourceId,
                    Option<ClassName>,
                    String,
                    String,
                    Option<String>,
                )>(c)?
                .into_iter()
                .map(|(id, class, method, args, ret)| {
                    (
                        TaintSource::MethodCall {
                            class,
                            method,
                            args,
                            ret,
                        },
                        id,
                    )
                });

                sources.extend(methods);

                Ok((ids, refs, sources))
            },
        )?;

        db.write(|c| -> db::Result<()> {
            query_exec!(
                update(run_info::table).set(run_info::started_at.eq(started_at)),
                c
            )?;
            Ok(())
        })?;

        let next_sink_id = refs.values().max().copied().unwrap_or(0) + 1;

        Ok(Self {
            db,
            next_sink_id,
            ids,
            refs,
            sources,
            factories,
            nodes: Vec::with_capacity(INSERT_BUFFER_SIZE),
            edges: Vec::with_capacity(INSERT_BUFFER_SIZE),
            graphs: Vec::with_capacity(INSERT_BUFFER_SIZE),
        })
    }

    fn allocate_sink_id(&mut self) -> i32 {
        let id = self.next_sink_id;
        self.next_sink_id = id + 1;
        id
    }

    fn get_sink_id(&mut self, borrowed: BorrowedSinkDef<'_>) -> db::Result<i32> {
        if let Some(known) = self.refs.get(&SinkDefWrapper::Borrowed(borrowed)) {
            return Ok(*known);
        }
        let new_id = self.allocate_sink_id();

        // TODO: These are probably better to chunk as well

        match borrowed {
            BorrowedSinkDef::Instruction(ins) => {
                let insert = InsertInstructionSink {
                    id: InstructionSinkId::new(new_id),
                    instruction: ins,
                };
                insert!(self.db, instruction_sinks::table, &insert)?;
            }
            BorrowedSinkDef::ExternalCall {
                class,
                name,
                signature,
            } => {
                let insert = InsertExternalCallSink {
                    id: ExternalCallSinkId::new(new_id),
                    class,
                    name,
                    signature,
                };
                insert!(self.db, external_call_sinks::table, &insert)?;
            }
            BorrowedSinkDef::ExternalField { class, name } => {
                let insert = InsertExternalFieldSink {
                    id: ExternalFieldSinkId::new(new_id),
                    class,
                    name,
                };
                insert!(self.db, external_field_sinks::table, &insert)?;
            }
        }

        let owned: SinkDefWrapper<'static> = SinkDefWrapper::Owned(borrowed.into());
        self.refs.insert(owned, new_id);

        Ok(new_id)
    }

    fn push_node(&mut self, id: NodeId, sink: RawSink, regs: String) -> db::Result<()> {
        let (kind, sink_id, graph_id) = match sink {
            RawSink::Call(id) => (SinkKind::Call, None, Some(id.id())),
            RawSink::Field(id) => (SinkKind::Field, None, Some(id.id())),
            RawSink::Phi => (SinkKind::Phi, None, None),
            RawSink::Array => (SinkKind::Array, None, None),
            RawSink::Instruction(ins) => {
                let borrowed = BorrowedSinkDef::Instruction(ins.as_str());
                (
                    SinkKind::Instruction,
                    Some(self.get_sink_id(borrowed)?),
                    None,
                )
            }
            RawSink::ExternalCall {
                class,
                name,
                signature,
            } => {
                let borrowed = BorrowedSinkDef::ExternalCall {
                    class: &class,
                    name: &name,
                    signature: &signature,
                };
                (
                    SinkKind::ExternalCall,
                    Some(self.get_sink_id(borrowed)?),
                    None,
                )
            }
            RawSink::ExternalField { class, name } => {
                let borrowed = BorrowedSinkDef::ExternalField {
                    class: &class,
                    name: &name,
                };
                (
                    SinkKind::ExternalField,
                    Some(self.get_sink_id(borrowed)?),
                    None,
                )
            }
        };

        let node = InsertNode {
            id,
            kind,
            sink_id,
            graph_id,
            regs,
        };

        self.nodes.push(node);
        Ok(())
    }

    pub fn on_action(&mut self, action: DatabaseAction) -> db::Result<()> {
        match action {
            DatabaseAction::FinishAnalysis { id } => self.db.write(|c| -> db::Result<()> {
                _ = query_exec!(
                    update(analyzed_methods::table.find(id))
                        .set(analyzed_methods::status.eq(MethodStatus::Done)),
                    c
                )?;
                Ok(())
            }),
            DatabaseAction::AddNode {
                id,
                sink,
                registers,
            } => {
                if self.nodes.len() >= INSERT_BUFFER_SIZE {
                    self.flush_nodes()?;
                }
                let registers = registers
                    .map(|it| it.into_iter().join(","))
                    .unwrap_or(String::new());
                self.push_node(id, sink, registers)?;
                Ok(())
            }
            DatabaseAction::AddEdge { src, dst, location } => {
                if self.edges.len() >= INSERT_BUFFER_SIZE {
                    self.flush_edges()?;
                }
                let edge = InsertEdge { src, dst, location };
                self.edges.push(edge);
                Ok(())
            }
            DatabaseAction::StartGraph {
                id,
                entry,
                analyzed_method,
                source,
            } => {
                if self.graphs.len() >= INSERT_BUFFER_SIZE {
                    self.flush_graphs()?;
                }

                self.graphs.push(InsertGraph {
                    id,
                    entry_node: entry,
                    analyzed_method,
                    source,
                });

                Ok(())
            }
        }
    }

    fn flush_nodes(&mut self) -> db::Result<()> {
        insert!(self.db, nodes::table, &self.nodes)?;
        self.nodes.clear();

        Ok(())
    }

    fn flush_edges(&mut self) -> db::Result<()> {
        // Ensure no foreign key issues
        if self.nodes.len() > 0 {
            self.flush_nodes()?;
        }

        insert_or_ignore!(self.db, edges::table, &self.edges)?;
        self.edges.clear();

        Ok(())
    }

    fn flush_graphs(&mut self) -> db::Result<()> {
        // Ensure no foreign key issues
        if self.nodes.len() > 0 {
            self.flush_nodes()?;
        }
        insert!(self.db, graphs::table, &self.graphs)?;
        self.graphs.clear();
        Ok(())
    }

    fn update_reachable_nodes(&self) -> db::Result<()> {
        self.db.write(update_reachable_nodes)
    }

    /// Finish a run, even one that was cancelled.
    ///
    /// Even cancelled runs have state that should be updated after a run, so this function should
    /// _always_ be called. It flushes any cached inserts and updates some secondary tables.
    pub fn finish_run(&mut self) -> db::Result<()> {
        if self.nodes.len() > 0 {
            self.flush_nodes()?;
        }

        if self.edges.len() > 0 {
            self.flush_edges()?;
        }

        if self.graphs.len() > 0 {
            self.flush_graphs()?;
        }

        self.update_reachable_nodes()?;

        // The other elements of sink_filters are inserted by triggers, but we can't create triggers
        // that reference the graph database without it already attached. `OR IGNORE` because this
        // can be run multiple times.
        const SINK_FILTER_UPDATE: &str = r#"
INSERT OR IGNORE INTO sink_filters (node, kind, class, name, args, ret)
SELECT n.id, 'call', c.name, m.name, m.args, m.ret
FROM nodes AS n
JOIN graph.methods AS m ON m.id = n.graph_id
JOIN graph.classes AS c ON c.id = m.class
WHERE n.kind = 'call';
"#;

        if let Err(e) = self.db.write(|c| -> db::Result<()> {
            _ = query!(sql_query(SINK_FILTER_UPDATE)).execute(c)?;
            Ok(())
        }) {
            log::warn!("failed to copy calls into the sink_filters table!: {e}");
        }

        self.rebuild_fts();

        Ok(())
    }

    /// Trigger a rebuild on the FTS5 virtual table
    ///
    /// This has to be run at the end of a run otherwise queries against the table won't return any
    /// data
    fn rebuild_fts(&self) {
        if self.db.db.check_fts5() {
            if let Err(e) = self.db.db.write(|c| -> db::Result<()> {
                _ = query!(sql_query(
                    "INSERT INTO sink_filters_fts(sink_filters_fts) VALUES ('rebuild');"
                ))
                .execute(c)?;
                Ok(())
            }) {
                log::warn!("failed to update sink filters fts table: {e}");
            }
        }
    }

    fn setup_fts5(db: &TaintAnalysisDbWriter) -> anyhow::Result<()> {
        let supports_fts5 = db.db.check_fts5();
        if !supports_fts5 {
            log::warn!("Host sqlite doesn't support FTS5, this will make filtering slower!");
            return Ok(());
        }

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

    fn create_new_run_meta(
        db: &'a TaintAnalysisDbWriter,
        info: &InsertRunInfo<'_>,
        factories: &'a IdFactories,
    ) -> db::Result<Self> {
        if let Err(e) = Self::setup_fts5(&db) {
            log::warn!("failed to setup FTS5: {e}");
        }

        let ids = HashMap::new();

        db.write(|conn| -> db::Result<()> {
            query_exec!(insert_into(run_info::table).values(info), conn)?;
            Ok(())
        })?;

        Ok(Self {
            db,
            ids,
            sources: HashMap::new(),
            refs: HashMap::new(),
            factories,
            next_sink_id: 1,

            nodes: Vec::with_capacity(INSERT_BUFFER_SIZE),
            edges: Vec::with_capacity(INSERT_BUFFER_SIZE),
            graphs: Vec::with_capacity(INSERT_BUFFER_SIZE),
        })
    }

    /// Get the [TaintSourceId] for the provided [TaintSource]
    ///
    /// This may add a new record to the database if it doesn't already exist
    pub fn id_for_source(&mut self, source: &TaintSource) -> db::Result<TaintSourceId> {
        if let Some(id) = self.sources.get(source) {
            return Ok(*id);
        }
        let id = self.factories.new_taint_source_id();

        self.db.write(|c| insert_source(c, id, source))?;
        self.sources.insert(source.clone(), id);

        Ok(id)
    }

    fn add_planned_method(
        &self,
        id: AnalyzedMethodId,
        planned: &PlannedMethod,
        buffer: &mut Vec<InsertAnalyzedMethod>,
    ) -> db::Result<()> {
        let ins = InsertAnalyzedMethod {
            id: id,
            method: planned.method,
            direct: matches!(planned.origin, UnresolvedOrigin::Direct),
            status: MethodStatus::Pending,
            error: None,
        };
        buffer.push(ins);
        if buffer.len() < INSERT_BUFFER_SIZE {
            return Ok(());
        }
        insert!(
            self.db,
            analyzed_methods::table,
            buffer as &Vec<InsertAnalyzedMethod>
        )?;
        buffer.clear();
        Ok(())
    }

    pub fn update_plan(
        &mut self,
        planned_iter: impl Iterator<Item = PlannedMethod>,
    ) -> db::Result<Vec<AnalyzedMethodId>> {
        let (lower_bound, _) = planned_iter.size_hint();
        self.ids.reserve(lower_bound);
        let mut analysis_methods = Vec::with_capacity(lower_bound);
        let mut methods = Vec::with_capacity(lower_bound);

        for planned in planned_iter {
            let id = match self.ids.get(&planned.method) {
                Some(v) => *v,
                None => {
                    let id = self.factories.new_analyzed_method_id();

                    self.add_planned_method(id, &planned, &mut methods)?;
                    id
                }
            };

            analysis_methods.push(id);

            self.ids.insert(planned.method, id);
        }

        if methods.len() > 0 {
            insert!(self.db, analyzed_methods::table, &methods)?;
        }

        Ok(analysis_methods)
    }

    /// Mark the analysis for `method` as failed with `reason`
    ///
    /// Does nothing if `method` was never planned, which shouldn't happen.
    pub fn mark_failed(&self, method: MethodId, reason: &str) -> db::Result<()> {
        let Some(&id) = self.ids.get(&method) else {
            return Ok(());
        };

        self.db.write(|c| -> db::Result<()> {
            query_exec!(
                update(analyzed_methods::table.find(id)).set((
                    analyzed_methods::status.eq(MethodStatus::Failed),
                    analyzed_methods::error.eq(Some(reason))
                )),
                c
            )?;
            Ok(())
        })
    }

    /// Record that the run finished, which is what separates a complete artifact from the
    /// file a cancelled run leaves behind
    pub fn mark_completed(self) -> db::Result<()> {
        self.db.write(|conn| -> db::Result<()> {
            query_exec!(
                update(run_info::table).set(run_info::completed.eq(true)),
                conn
            )?;
            Ok(())
        })
    }
}

fn insert_source(
    conn: &mut SqliteConnection,
    id: TaintSourceId,
    source: &TaintSource,
) -> db::Result<()> {
    let kind = match source {
        TaintSource::Param { .. } => SourceKind::Param,
        TaintSource::MethodCall { .. } => SourceKind::Call,
        TaintSource::Field { .. } => SourceKind::Field,
    };
    let source_id = match source {
        TaintSource::Param { register } => {
            let values = InsertSourceParam {
                register: i32::from(*register),
            };
            query!(insert_into(source_params::table)
                .values(&values)
                .returning(source_params::id))
            .get_result::<i32>(conn)?
        }
        TaintSource::MethodCall {
            class,
            method,
            args,
            ret,
        } => {
            let values = InsertSourceCall {
                class: class.as_ref(),
                name: method,
                args,
                ret: ret.as_deref(),
            };
            query!(insert_into(source_calls::table)
                .values(&values)
                .returning(source_calls::id))
            .get_result::<i32>(conn)?
        }
        TaintSource::Field { class, name } => {
            let values = InsertSourceField { class, name };
            query!(insert_into(source_fields::table)
                .values(&values)
                .returning(source_fields::id))
            .get_result::<i32>(conn)?
        }
    };

    let values = InsertTaintSource {
        id,
        source_id,
        kind,
    };
    query_exec!(insert_into(taint_sources::table).values(&values), conn)?;

    Ok(())
}

fn do_update_reachable_nodes(conn: &mut SqliteConnection) -> db::Result<()> {
    // Seed the table with subgraphs that were created, and finished, in this run
    let q = sql_query(
        r#"
INSERT INTO staging_reachable_nodes (graph, node, parent, depth, location)
SELECT g.id, g.entry_node, NULL, 0, am.method
FROM graphs AS g 
JOIN analyzed_methods AS am
    ON g.analyzed_method = am.id
WHERE am.status = 'done'
      AND
      g.id NOT IN (SELECT graph FROM reachable_nodes)
"#,
    );

    let res = query!(q).execute(conn)?;

    if res == 0 {
        return Ok(());
    }

    let mut depth = 0;

    // Keep adding new rows with increasing depth until we add no more rows
    loop {
        let res = query!(sql_query(
            // A node can be reached by several edges at the same depth, and only one of them
            // becomes its parent. Prefer the edge whose source node *is* the method the child
            // sits in: a call recorded inside `LK/n;->c` belongs under the node for `LK/n;->c`,
            // not beside it under whatever else reached it at the same distance. Failing that,
            // order by location and then source so the choice is at least deterministic.
            r#"INSERT OR IGNORE INTO staging_reachable_nodes (graph, node, parent, depth, location)
SELECT graph, dst, parent, depth, location
FROM (
    SELECT
        st.graph                AS graph,
        e.dst                   AS dst,
        st.node                 AS parent,
        ?1 + 1                  AS depth,
        e.location              AS location,
        ROW_NUMBER() OVER (
            PARTITION BY st.graph, e.dst
            ORDER BY
                COALESCE(pn.kind = 'call' AND pn.graph_id = e.location, 0) DESC,
                e.location,
                st.node
        )                       AS pick
    FROM staging_reachable_nodes AS st
    JOIN edges AS e ON e.src = st.node
    JOIN nodes AS pn ON pn.id = st.node
    WHERE st.depth = ?1
)
WHERE pick = 1"#
        )
        .bind::<Integer, _>(depth))
        .execute(conn)?;

        if res == 0 {
            break;
        }

        depth += 1;
    }

    // Transfer the staging table to the real table

    query!(sql_query(
        r#"
INSERT INTO reachable_nodes(graph, node, parent, depth, location)
SELECT graph, node, parent, depth, location FROM staging_reachable_nodes ORDER BY depth
"#
    ))
    .execute(conn)?;

    // Update the graph metadata for the new entries

    query!(sql_query(
        r#"
INSERT INTO graph_metadata (graph, analyzed_method, nphi, depth, size)
SELECT
    st.graph,
    g.analyzed_method,
    SUM(CASE WHEN n.kind = 'phi' THEN 1 ELSE 0 END),
    MAX(st.depth) + 1,
    COUNT(st.node)
FROM staging_reachable_nodes AS st
JOIN nodes AS n
    ON n.id = st.node
JOIN graphs AS g
    ON g.id = st.graph
GROUP BY st.graph;
"#
    ))
    .execute(conn)?;

    Ok(())
}

fn update_reachable_nodes(conn: &mut SqliteConnection) -> db::Result<()> {
    conn.batch_execute(
        r#"
CREATE TEMPORARY TABLE staging_reachable_nodes(
    graph       INTEGER NOT NULL,
    node        INTEGER NOT NULL,
    parent      INTEGER,
    depth       INTEGER NOT NULL,
    location    INTEGER NOT NULL,

    PRIMARY KEY (graph, node)
)"#,
    )?;

    let res = do_update_reachable_nodes(conn);

    let drop_res = conn.batch_execute("DROP TABLE staging_reachable_nodes");

    if res.is_ok() {
        drop_res?;
    }

    res
}
