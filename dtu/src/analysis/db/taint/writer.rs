use std::collections::HashMap;

use diesel::{insert_into, sql_query, SqliteConnection};
use diesel::{prelude::*, update};

use super::db::SinkDef;
use super::models::*;
use super::schema::{
    analyzed_methods, call_graph_chains, external_sinks, field_sinks, instruction_sinks, routes,
    run_info, sinks, source_calls, source_fields, source_params, taint_sources,
};
use crate::analysis::db::taint::db::{GraphTaintAnalysisDb, UnresolvedOrigin};
use crate::analysis::db::TaintAnalysisDb;
use crate::analysis::taint::{MethodTaint, TaintSink, TaintSinkKind, TaintSource};
use crate::db::graph::models::MethodId;
use crate::db::graph::GraphDatabase;
use crate::db::{self, query, query_exec, DatabaseId};
use crate::utils::{unix_now, ClassName};
use crate::VERSION;

/// Rows per insert statement
///
/// Six columns per sink, so this stays well inside SQLITE_MAX_VARIABLE_NUMBER.
const CHUNK: usize = 2000;

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

/// A method the run intends to analyze, known before any analysis happens
pub struct PlannedMethod {
    pub method: MethodId,
    pub origin: UnresolvedOrigin,
}

/// Streams the result of one taint command into an artifact.
///
/// Every method is written in its own transaction as it completes, so a run that is
/// cancelled or dies leaves a file describing what it got through. Ids are assigned here
/// rather than by sqlite, since a route is referenced by its sinks.
pub struct TaintWriter<'a> {
    pub(crate) db: &'a GraphTaintAnalysisDb,
    ids: HashMap<MethodId, AnalyzedMethodId>,
    refs: HashMap<SinkDef, SinkId>,
    next_source: i32,
    next_route: i32,
    next_sink: i32,
}

impl<'a> TaintWriter<'a> {
    /// Write a new run info if one doesn't exist and update one that already does
    pub fn update_run_meta(
        db: &'a GraphTaintAnalysisDb,
        info: &InsertRunInfo<'_>,
        planned: &[PlannedMethod],
    ) -> db::Result<Self> {
        let have_run_info = db
            .query(|c| {
                run_info::table
                    .select(run_info::started_at)
                    .get_result::<i64>(c)
                    .optional()
            })?
            .is_some();

        // Unconditional temporary trigger that can't exist in the migrations because it references
        // the graph
        let method_trigger = r#"CREATE TEMP TRIGGER update_sink_filters_method
AFTER INSERT ON sinks
WHEN new.kind = 'call'
BEGIN
    INSERT INTO sink_filters (route, idx, kind, class, name, args, ret)
    SELECT
        new.route,
        new.idx,
        new.kind,
        classes.name,
        methods.name,
        methods.args,
        methods.ret
    FROM methods
    JOIN classes ON classes.id = methods.class
    WHERE methods.id = new.method_id;
END;"#;

        db.write(|c| -> db::Result<()> {
            _ = query!(sql_query(method_trigger)).execute(c)?;
            Ok(())
        })?;

        if !have_run_info {
            return Self::create_new_run_meta(db, info, planned);
        }

        Self::update_existing_run_meta(db)
    }

    fn update_existing_run_meta(db: &'a GraphTaintAnalysisDb) -> db::Result<Self> {
        let started_at = unix_now()
            .map_err(|_| db::Error::Generic("failed to get the unix timestamp".into()))?;

        let (ids, refs, next_source, next_route, next_sink) = db.query(
            |c| -> db::Result<(
                HashMap<MethodId, AnalyzedMethodId>,
                HashMap<SinkDef, SinkId>,
                i32,
                i32,
                i32,
            )> {
                let ids = HashMap::from_iter(
                    query!(analyzed_methods::table
                        .select((analyzed_methods::method, analyzed_methods::id)))
                    .get_results::<(MethodId, AnalyzedMethodId)>(c)?
                    .iter()
                    .copied(),
                );

                let mut refs: HashMap<SinkDef, SinkId> = HashMap::new();

                refs.extend(
                    query!(instruction_sinks::table
                        .select((instruction_sinks::instruction, instruction_sinks::id)))
                    .get_results::<(String, InstructionSinkId)>(c)?
                    .into_iter()
                    .map(|(instruction, id)| {
                        (SinkDef::Instruction(instruction), SinkId::Instruction(id))
                    }),
                );

                refs.extend(
                    query!(field_sinks::table.select((
                        field_sinks::class,
                        field_sinks::name,
                        field_sinks::id
                    )))
                    .get_results::<(ClassName, String, FieldSinkId)>(c)?
                    .into_iter()
                    .map(|(class, name, id)| (SinkDef::Field { class, name }, SinkId::Field(id))),
                );

                refs.extend(
                    query!(external_sinks::table.select((
                        external_sinks::class,
                        external_sinks::name,
                        external_sinks::signature,
                        external_sinks::id,
                    )))
                    .get_results::<(ClassName, String, String, ExternalSinkId)>(c)?
                    .into_iter()
                    .map(|(class, name, signature, id)| {
                        (
                            SinkDef::External {
                                class,
                                name,
                                signature,
                            },
                            SinkId::External(id),
                        )
                    }),
                );

                // Adding 1 to all of these because IDs start at 1 not 0
                let nsources = taint_sources::table.count().get_result::<i64>(c)? + 1;
                let nroutes = routes::table.count().get_result::<i64>(c)? + 1;
                let nsinks = sinks::table.count().get_result::<i64>(c)? + 1;
                Ok((ids, refs, nsources as i32, nroutes as i32, nsinks as i32))
            },
        )?;

        db.write(|c| -> db::Result<()> {
            query_exec!(
                update(run_info::table).set(run_info::started_at.eq(started_at)),
                c
            )?;
            Ok(())
        })?;

        Ok(Self {
            db,
            ids,
            refs,
            next_source,
            next_route,
            next_sink,
        })
    }

    /// Trigger a rebuild on the FTS5 virtual table
    ///
    /// This has to be run at the end of a run otherwise queries against the table won't return any
    /// data
    pub fn rebuild_fts(&self) {
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

    fn setup_fts5(db: &GraphTaintAnalysisDb) -> anyhow::Result<()> {
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
        db: &'a GraphTaintAnalysisDb,
        info: &InsertRunInfo<'_>,
        planned: &[PlannedMethod],
    ) -> db::Result<Self> {
        if let Err(e) = Self::setup_fts5(&db) {
            log::warn!("failed to setup FTS5: {e}");
        }

        let mut ids = HashMap::with_capacity(planned.len());
        let mut methods = Vec::with_capacity(planned.len());
        let mut chains = Vec::new();

        for (idx, plan) in planned.iter().enumerate() {
            let id = AnalyzedMethodId::from_id(idx as i32 + 1);
            ids.insert(plan.method, id);
            methods.push(InsertInitialAnalyzedMethod {
                id,
                method: plan.method,
                direct: matches!(plan.origin, UnresolvedOrigin::Direct),
            });

            if let UnresolvedOrigin::CallGraph { chains: found } = &plan.origin {
                for (chain, hops) in found.iter().enumerate() {
                    for (hop, method) in hops.iter().enumerate() {
                        chains.push(InsertCallGraphChain {
                            analyzed_method: id,
                            chain: ChainId::new(chain as i32),
                            idx: hop as i32,
                            method: *method,
                        });
                    }
                }
            }
        }

        db.write(|conn| -> db::Result<()> {
            query_exec!(insert_into(run_info::table).values(info), conn)?;
            for batch in methods.chunks(CHUNK) {
                query_exec!(insert_into(analyzed_methods::table).values(batch), conn)?;
            }
            for batch in chains.chunks(CHUNK) {
                query_exec!(insert_into(call_graph_chains::table).values(batch), conn)?;
            }
            Ok(())
        })?;

        Ok(Self {
            db,
            ids,
            refs: HashMap::new(),
            next_sink: 1,
            next_source: 1,
            next_route: 1,
        })
    }

    /// Record everything found in one method and mark it done
    pub fn write_method(&mut self, taint: &MethodTaint) -> db::Result<()> {
        let analyzed = self.analyzed_id(taint.method)?;
        let Self {
            db,
            refs,
            next_source,
            next_route,
            next_sink,
            ..
        } = self;

        // Refs discovered here are only merged into the interner once the transaction
        // commits, otherwise a rollback would leave ids pointing at rows that never existed
        let mut new_refs: HashMap<SinkDef, SinkId> = HashMap::new();
        let mut route_count = 0i32;

        db.write(|conn| -> db::Result<()> {
            for entry in &taint.taint {
                let source = TaintSourceId::from_id(*next_source);
                *next_source += 1;
                insert_source(conn, source, analyzed, &entry.source)?;

                for route in &entry.paths {
                    // Just stop early for empty routes, we added the taint source already but it
                    // could end up leading nowhere. If we don't do this, `rount_count` will be
                    // incorrect and queries will return unexpected results.
                    if route.is_empty() {
                        continue;
                    }
                    let id = RouteId::from_id(*next_route);
                    *next_route += 1;
                    route_count += 1;

                    let values = InsertRoute {
                        id,
                        source,
                        incomplete: route.incomplete,
                        phi_count: route
                            .iter()
                            .filter(|it| matches!(it.kind, TaintSinkKind::Phi))
                            .count() as i32,
                    };
                    query_exec!(insert_into(routes::table).values(&values), conn)?;

                    let mut rows = Vec::with_capacity(route.len().min(CHUNK));
                    for (idx, sink) in route.iter().enumerate() {
                        rows.push(sink_row(
                            conn,
                            id,
                            idx,
                            sink,
                            refs,
                            &mut new_refs,
                            next_sink,
                        )?);
                        if rows.len() == CHUNK {
                            insert_sinks(conn, &rows)?;
                            rows.clear();
                        }
                    }
                    insert_sinks(conn, &rows)?;
                }
            }

            query_exec!(
                update(analyzed_methods::table.find(analyzed)).set((
                    analyzed_methods::status.eq(MethodStatus::Done),
                    analyzed_methods::route_count.eq(route_count),
                )),
                conn
            )?;
            Ok(())
        })?;

        self.refs.extend(new_refs);
        Ok(())
    }

    /// Mark a method done that the analysis reached nothing in
    pub fn mark_done(&mut self, method: MethodId) -> db::Result<()> {
        let analyzed = self.analyzed_id(method)?;
        self.db.write(|conn| -> db::Result<()> {
            query_exec!(
                update(analyzed_methods::table.find(analyzed))
                    .set(analyzed_methods::status.eq(MethodStatus::Done)),
                conn
            )?;
            Ok(())
        })
    }

    pub fn mark_failed(&mut self, method: MethodId, reason: &str) -> db::Result<()> {
        let analyzed = self.analyzed_id(method)?;
        self.db.write(|conn| -> db::Result<()> {
            query_exec!(
                update(analyzed_methods::table.find(analyzed)).set((
                    analyzed_methods::status.eq(MethodStatus::Failed),
                    analyzed_methods::error.eq(reason),
                )),
                conn
            )?;
            Ok(())
        })
    }

    /// Record that the run finished, which is what separates a complete artifact from the
    /// file a cancelled run leaves behind
    pub fn finish(self) -> db::Result<()> {
        self.db.write(|conn| -> db::Result<()> {
            query_exec!(
                update(run_info::table).set(run_info::completed.eq(true)),
                conn
            )?;
            Ok(())
        })
    }

    fn analyzed_id(&self, method: MethodId) -> db::Result<AnalyzedMethodId> {
        self.ids.get(&method).copied().ok_or_else(|| {
            db::Error::Generic(format!("method {method} was not part of the run's plan"))
        })
    }
}

fn insert_source(
    conn: &mut SqliteConnection,
    id: TaintSourceId,
    analyzed: AnalyzedMethodId,
    source: &TaintSource,
) -> db::Result<()> {
    let kind = match source {
        TaintSource::Param { .. } => SourceKind::Param,
        TaintSource::MethodCall { .. } => SourceKind::Call,
        TaintSource::Field { .. } => SourceKind::Field,
    };
    let values = InsertTaintSource {
        id,
        analyzed_method: analyzed,
        kind,
    };
    query_exec!(insert_into(taint_sources::table).values(&values), conn)?;

    match source {
        TaintSource::Param { register } => {
            let values = InsertSourceParam {
                source: id,
                register: i32::from(*register),
            };
            query_exec!(insert_into(source_params::table).values(&values), conn)?;
        }
        TaintSource::MethodCall {
            class,
            method,
            args,
            ret,
        } => {
            let values = InsertSourceCall {
                source: id,
                class: class.as_ref(),
                name: method,
                args,
                ret: ret.as_deref(),
            };
            query_exec!(insert_into(source_calls::table).values(&values), conn)?;
        }
        TaintSource::Field { class, name } => {
            let values = InsertSourceField {
                source: id,
                class,
                name,
            };
            query_exec!(insert_into(source_fields::table).values(&values), conn)?;
        }
    }
    Ok(())
}

fn insert_sinks(conn: &mut SqliteConnection, rows: &[InsertSink]) -> db::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    query_exec!(insert_into(sinks::table).values(rows), conn)?;
    Ok(())
}

fn sink_row(
    conn: &mut SqliteConnection,
    route: RouteId,
    idx: usize,
    sink: &TaintSink,
    refs: &HashMap<SinkDef, SinkId>,
    new_refs: &mut HashMap<SinkDef, SinkId>,
    next_sink: &mut i32,
) -> db::Result<InsertSink> {
    let (kind, method_id, key) = match &sink.kind {
        TaintSinkKind::Phi => (SinkKind::Phi, None, None),
        TaintSinkKind::Array => (SinkKind::Array, None, None),
        TaintSinkKind::MethodCall { method } => (SinkKind::Call, Some(*method), None),
        TaintSinkKind::Instruction { opcode } => (
            SinkKind::Instruction,
            None,
            Some(SinkDef::Instruction(opcode.clone())),
        ),
        TaintSinkKind::Field { class, name } => (
            SinkKind::Field,
            None,
            Some(SinkDef::Field {
                class: class.clone(),
                name: name.clone(),
            }),
        ),
        TaintSinkKind::ExternalCall { class, name, args } => (
            SinkKind::External,
            None,
            Some(SinkDef::External {
                class: class.clone(),
                name: name.clone(),
                signature: args.clone(),
            }),
        ),
    };

    let ref_ = match key {
        None => None,
        Some(key) => Some(intern_ref(conn, key, refs, new_refs, next_sink)?),
    };

    let sink_id = ref_.map(SinkId::raw);

    Ok(InsertSink {
        route,
        idx: idx as i32,
        location: sink.location,
        kind,
        sink_id,
        method_id,
    })
}

fn add_instruction_ref(
    conn: &mut SqliteConnection,
    instruction: &str,
    sink_id: i32,
) -> db::Result<InstructionSinkId> {
    let id = InstructionSinkId::new(sink_id);
    query_exec!(
        insert_into(instruction_sinks::table).values(InsertInstructionSink { id, instruction }),
        conn
    )?;
    Ok(id)
}

fn add_field_ref(
    conn: &mut SqliteConnection,
    class: &ClassName,
    name: &str,
    sink_id: i32,
) -> db::Result<FieldSinkId> {
    let id = FieldSinkId::new(sink_id);
    query_exec!(
        insert_into(field_sinks::table).values(InsertFieldSink { id, class, name }),
        conn
    )?;
    Ok(id)
}

fn add_external_ref(
    conn: &mut SqliteConnection,
    class: &ClassName,
    name: &str,
    signature: &str,
    sink_id: i32,
) -> db::Result<ExternalSinkId> {
    let id = ExternalSinkId::new(sink_id);
    query_exec!(
        insert_into(external_sinks::table).values(InsertExternalSink {
            id,
            class,
            name,
            signature,
        }),
        conn
    )?;
    Ok(id)
}

fn intern_ref(
    conn: &mut SqliteConnection,
    key: SinkDef,
    refs: &HashMap<SinkDef, SinkId>,
    new_refs: &mut HashMap<SinkDef, SinkId>,
    next_sink: &mut i32,
) -> db::Result<SinkId> {
    if let Some(id) = refs.get(&key).or_else(|| new_refs.get(&key)) {
        return Ok(*id);
    }

    let raw_sink_id = *next_sink;
    *next_sink += 1;

    let id = match &key {
        SinkDef::Instruction(ins) => {
            add_instruction_ref(conn, ins, raw_sink_id).map(SinkId::Instruction)?
        }
        SinkDef::Field { class, name } => {
            add_field_ref(conn, class, name, raw_sink_id).map(SinkId::Field)?
        }
        SinkDef::External {
            class,
            name,
            signature,
        } => add_external_ref(conn, class, name, signature, raw_sink_id).map(SinkId::External)?,
    };

    new_refs.insert(key, id);
    Ok(id)
}
