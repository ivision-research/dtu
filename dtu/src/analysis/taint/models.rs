use diesel::prelude::*;

use super::schema::{
    _hidden_analyzed_methods, _hidden_graphs, analyzed_methods, edges,
    external_call_sinks, external_field_sinks, graphs, instruction_sinks, nodes, reachable_nodes,
    run_info, source_calls, source_fields, source_params, taint_sources,
};
use dtu_proc_macro::sql_db_row;

use crate::db::graph::models::{FieldId, MethodId};
use crate::db::macros::{database_id, text_enum};
use crate::db::DatabaseId;
use crate::utils::ClassName;

text_enum!(SourceKind {
    Param => "param",
    Call => "call",
    Field => "field",
});

text_enum!(SinkKind {
    Call => "call",
    Field => "field",
    ExternalField => "ext-field",
    ExternalCall => "ext-call",
    Instruction => "instr",
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
database_id!(SubgraphId, "Identifies one subgraph in the graph");
database_id!(NodeId, "Identifies a single node in the database");

database_id!(TaintSourceId, "Identifies a seed of one analyzed method");
database_id!(FieldSourceId, "Identifies a field taint source");
database_id!(MethodSourceId, "Identifies a method taint source");
database_id!(RegisterSourceId, "Identifies a register taint source");

database_id!(InstructionSinkId, "Identifies an instruction sink");
database_id!(ExternalCallSinkId, "Identifies an external call sink");
database_id!(ExternalFieldSinkId, "Identifies an external field sink");

/// Values that are the sink of a [Node]
///
/// This combines both [GraphReference] and [SinkId]
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum NodeSink {
    Graph(GraphReference),
    Sink(SinkId),
}

macro_rules! enum_from {
    ($enum:ident, $node:ident, $inner:ident) => {
        impl From<$inner> for $enum {
            fn from(value: $inner) -> Self {
                Self::$node(value.into())
            }
        }
    };
}

enum_from!(NodeSink, Graph, GraphReference);
enum_from!(NodeSink, Sink, SinkId);
enum_from!(NodeSink, Graph, FieldId);
enum_from!(NodeSink, Graph, MethodId);
enum_from!(NodeSink, Sink, ExternalFieldSinkId);
enum_from!(NodeSink, Sink, ExternalCallSinkId);
enum_from!(NodeSink, Sink, InstructionSinkId);

/// Values that are an external view into the graph database
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum GraphReference {
    Field(FieldId),
    Method(MethodId),
}

enum_from!(GraphReference, Field, FieldId);
enum_from!(GraphReference, Method, MethodId);

/// Values for sinks defined in the taint database itself
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum SinkId {
    ExternalField(ExternalFieldSinkId),
    ExternalCall(ExternalCallSinkId),
    Instruction(InstructionSinkId),
}

enum_from!(SinkId, ExternalField, ExternalFieldSinkId);
enum_from!(SinkId, ExternalCall, ExternalCallSinkId);
enum_from!(SinkId, Instruction, InstructionSinkId);

impl Into<i32> for SinkId {
    fn into(self) -> i32 {
        match self {
            Self::ExternalField(v) => v.into(),
            Self::ExternalCall(v) => v.into(),
            Self::Instruction(v) => v.into(),
        }
    }
}

impl SinkId {
    pub fn raw(self) -> i32 {
        match self {
            Self::ExternalField(v) => v.raw(),
            Self::Instruction(v) => v.raw(),
            Self::ExternalCall(v) => v.raw(),
        }
    }
}

#[sql_db_row]
pub struct ReachableNode {
    pub graph: SubgraphId,
    pub node: NodeId,
    pub parent: Option<NodeId>,
    pub depth: i32,
}

#[sql_db_row]
#[diesel(table_name = _hidden_analyzed_methods)]
pub struct HiddenAnalyzedMethod {
    pub analyzed_method: AnalyzedMethodId,
}

#[sql_db_row]
#[diesel(table_name = _hidden_graphs)]
pub struct HiddenGraph {
    pub graph: SubgraphId,
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

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct AnalyzedMethod {
    pub id: AnalyzedMethodId,
    pub method: MethodId,
    pub direct: bool,
    pub status: MethodStatus,
    pub error: Option<String>,
}

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct TaintSource {
    pub id: TaintSourceId,
    pub kind: SourceKind,
    pub source_id: i32,
}

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct Graph {
    pub id: SubgraphId,
    pub analyzed_method: AnalyzedMethodId,
    pub entry_node: NodeId,
    pub source: TaintSourceId,
}

#[sql_db_row]
pub struct SourceParam {
    pub id: RegisterSourceId,
    pub register: i32,
}

#[sql_db_row]
pub struct SourceCall {
    pub id: MethodSourceId,
    pub class: Option<ClassName>,
    pub name: String,
    pub args: String,
    pub ret: Option<String>,
}

#[sql_db_row]
pub struct SourceField {
    pub id: FieldSourceId,
    pub class: ClassName,
    pub name: String,
}

#[sql_db_row]
#[dtu(insert_keep_id, insert_owned)]
pub struct Node {
    pub id: NodeId,
    pub kind: SinkKind,
    pub sink_id: Option<i32>,
    /// This may either be a method or a field depending on [Self::kind]
    pub graph_id: Option<i32>,
    pub regs: String,
}

#[derive(Insertable)]
#[diesel(table_name = edges)]
pub struct InsertEdge {
    pub src: NodeId,
    pub dst: NodeId,
    /// The method `dst` sits in, see the `edges` table comment for why it lives here
    pub location: MethodId,
}

impl Node {
    pub fn get_sink(&self) -> Option<NodeSink> {
        Some(match self.kind {
            SinkKind::Phi | SinkKind::Array => return None,
            SinkKind::Call => NodeSink::from(MethodId::from_id(self.graph_id?)),
            SinkKind::Field => NodeSink::from(FieldId::from_id(self.graph_id?)),
            SinkKind::ExternalCall => NodeSink::from(ExternalCallSinkId::from_id(self.sink_id?)),
            SinkKind::ExternalField => NodeSink::from(ExternalFieldSinkId::from_id(self.sink_id?)),
            SinkKind::Instruction => NodeSink::from(InstructionSinkId::from_id(self.sink_id?)),
        })
    }
}

#[sql_db_row]
#[dtu(insert_keep_id)]
pub struct ExternalFieldSink {
    pub id: ExternalFieldSinkId,
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
pub struct ExternalCallSink {
    pub id: ExternalCallSinkId,
    pub class: ClassName,
    pub name: String,
    pub signature: String,
}
