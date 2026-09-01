use diesel::prelude::*;

use super::schema::{
    _hidden_analyzed_methods, _hidden_routes, analyzed_methods, call_graph_chains, external_sinks,
    field_sinks, instruction_sinks, routes, run_info, sinks, source_calls, source_fields,
    source_params, taint_sources,
};
use dtu_proc_macro::sql_db_row;

use crate::db::graph::models::MethodId;
use crate::db::macros::{database_id, text_enum};
use crate::utils::ClassName;

text_enum!(SourceKind {
    Param => "param",
    Call => "call",
    Field => "field",
});

text_enum!(SinkKind {
    Call => "call",
    Field => "field",
    External => "external",
    Instruction => "instruction",
    Phi => "phi",
    Array => "array",
});

text_enum!(MethodStatus {
    Pending => "pending",
    Failed => "failed",
    Done => "done",
});

database_id!(
    AnalyzedMethodId,
    "Identifies a method the run planned to analyze"
);
database_id!(TaintSourceId, "Identifies a seed of one analyzed method");
database_id!(RouteId, "Identifies one route from a seed");
database_id!(FieldSinkId, "Identifies a field sink");
database_id!(InstructionSinkId, "Identifies an instruction sink");
database_id!(ExternalSinkId, "Identifies an external sink");
database_id!(ChainId, "Identifies a call graph chain");

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum SinkId {
    Field(FieldSinkId),
    External(ExternalSinkId),
    Instruction(InstructionSinkId),
}

impl Into<i32> for SinkId {
    fn into(self) -> i32 {
        match self {
            Self::Field(v) => v.into(),
            Self::External(v) => v.into(),
            Self::Instruction(v) => v.into(),
        }
    }
}

impl From<FieldSinkId> for SinkId {
    fn from(value: FieldSinkId) -> Self {
        Self::Field(value)
    }
}

impl From<InstructionSinkId> for SinkId {
    fn from(value: InstructionSinkId) -> Self {
        Self::Instruction(value)
    }
}

impl From<ExternalSinkId> for SinkId {
    fn from(value: ExternalSinkId) -> Self {
        Self::External(value)
    }
}

impl SinkId {
    pub fn raw(self) -> i32 {
        match self {
            Self::Field(v) => v.raw(),
            Self::Instruction(v) => v.raw(),
            Self::External(v) => v.raw(),
        }
    }
}

#[sql_db_row]
#[diesel(table_name = _hidden_analyzed_methods)]
pub struct HiddenAnalyzedMethod {
    pub analyzed_method: AnalyzedMethodId,
}

#[sql_db_row]
#[diesel(table_name = _hidden_routes)]
pub struct HiddenRoute {
    pub route: RouteId,
}

#[sql_db_row]
#[diesel(table_name = run_info)]
pub struct RunInfo {
    pub options: String,
    pub schema_version: i32,
    pub graph_built_at: i64,
    pub dtu_version: String,
    pub started_at: i64,
    pub completed: bool,
}

#[derive(Insertable)]
#[diesel(table_name = analyzed_methods)]
pub struct InsertInitialAnalyzedMethod {
    pub id: AnalyzedMethodId,
    pub method: MethodId,
    pub direct: bool,
}

#[sql_db_row]
pub struct AnalyzedMethod {
    pub id: AnalyzedMethodId,
    pub method: MethodId,
    pub direct: bool,
    pub status: MethodStatus,
    pub error: Option<String>,
    pub route_count: i32,
}

#[sql_db_row]
pub struct CallGraphChain {
    pub analyzed_method: AnalyzedMethodId,
    pub chain: ChainId,
    pub idx: i32,
    pub method: MethodId,
}

#[derive(Insertable)]
#[diesel(table_name = taint_sources)]
pub struct InsertTaintSource {
    pub id: TaintSourceId,
    pub analyzed_method: AnalyzedMethodId,
    pub kind: SourceKind,
}

#[sql_db_row]
pub struct SourceParam {
    pub source: TaintSourceId,
    pub register: i32,
}

#[sql_db_row]
pub struct SourceCall {
    pub source: TaintSourceId,
    pub class: Option<ClassName>,
    pub name: String,
    pub args: String,
    pub ret: Option<String>,
}

#[sql_db_row]
pub struct SourceField {
    pub source: TaintSourceId,
    pub class: ClassName,
    pub name: String,
}

#[derive(Insertable)]
#[diesel(table_name = routes)]
pub struct InsertRoute {
    pub id: RouteId,
    pub source: TaintSourceId,
    pub incomplete: bool,
    pub phi_count: i32,
}

#[sql_db_row]
pub struct Sink {
    pub route: RouteId,
    pub idx: i32,
    pub location: MethodId,
    pub kind: SinkKind,
    pub sink_id: Option<i32>,
    pub method_id: Option<MethodId>,
}

impl Sink {
    pub fn get_id(&self) -> Option<SinkId> {
        let id = self.sink_id?;
        Some(match self.kind {
            SinkKind::Phi | SinkKind::Array | SinkKind::Call => return None,
            SinkKind::External => SinkId::External(id.into()),
            SinkKind::Instruction => SinkId::Instruction(id.into()),
            SinkKind::Field => SinkId::Field(id.into()),
        })
    }
}

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct FieldSink {
    pub id: FieldSinkId,
    pub class: ClassName,
    pub name: String,
}

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct InstructionSink {
    pub id: InstructionSinkId,
    pub instruction: String,
}

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct ExternalSink {
    pub id: ExternalSinkId,
    pub class: ClassName,
    pub name: String,
    pub signature: String,
}
