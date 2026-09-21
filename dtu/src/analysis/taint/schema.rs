// @generated automatically by Diesel CLI.

diesel::table! {
    _hidden_analyzed_methods (analyzed_method) {
        analyzed_method -> Integer,
    }
}

diesel::table! {
    _hidden_graphs (graph) {
        graph -> Integer,
    }
}

diesel::table! {
    analyzed_methods (id) {
        id -> Integer,
        method -> Integer,
        direct -> Bool,
        status -> Text,
        error -> Nullable<Text>,
    }
}

diesel::table! {
    edges (src, dst, location) {
        src -> Integer,
        dst -> Integer,
        location -> Integer,
    }
}

diesel::table! {
    external_call_sinks (id) {
        id -> Integer,
        class -> Text,
        name -> Text,
        signature -> Text,
    }
}

diesel::table! {
    external_field_sinks (id) {
        id -> Integer,
        class -> Text,
        name -> Text,
    }
}

diesel::table! {
    graph_metadata (graph) {
        graph -> Integer,
        analyzed_method -> Integer,
        nphi -> Integer,
        depth -> Integer,
        size -> Integer,
    }
}

diesel::table! {
    graphs (id) {
        id -> Integer,
        analyzed_method -> Integer,
        entry_node -> Integer,
        source -> Integer,
    }
}

diesel::table! {
    instruction_sinks (id) {
        id -> Integer,
        instruction -> Text,
    }
}

diesel::table! {
    nodes (id) {
        id -> Integer,
        kind -> Text,
        sink_id -> Nullable<Integer>,
        graph_id -> Nullable<Integer>,
        regs -> Text,
    }
}

diesel::table! {
    reachable_nodes (graph, node) {
        graph -> Integer,
        node -> Integer,
        parent -> Nullable<Integer>,
        depth -> Integer,
        location -> Integer,
    }
}

diesel::table! {
    run_info (rowid) {
        rowid -> Integer,
        options -> Text,
        schema_version -> Integer,
        graph_built_at -> BigInt,
        dtu_version -> Text,
        started_at -> BigInt,
        completed -> Bool,
    }
}

diesel::table! {
    sink_filters (id) {
        id -> Integer,
        node -> Integer,
        kind -> Text,
        class -> Nullable<Text>,
        name -> Nullable<Text>,
        args -> Nullable<Text>,
        ret -> Nullable<Text>,
    }
}

diesel::table! {
    source_calls (id) {
        id -> Integer,
        class -> Nullable<Text>,
        name -> Text,
        args -> Text,
        ret -> Nullable<Text>,
    }
}

diesel::table! {
    source_fields (id) {
        id -> Integer,
        class -> Text,
        name -> Text,
    }
}

diesel::table! {
    source_params (id) {
        id -> Integer,
        register -> Integer,
    }
}

diesel::table! {
    taint_sources (id) {
        id -> Integer,
        kind -> Text,
        source_id -> Integer,
    }
}

diesel::joinable!(_hidden_analyzed_methods -> analyzed_methods (analyzed_method));
diesel::joinable!(_hidden_graphs -> graphs (graph));
diesel::joinable!(graph_metadata -> analyzed_methods (analyzed_method));
diesel::joinable!(graph_metadata -> graphs (graph));
diesel::joinable!(graphs -> analyzed_methods (analyzed_method));
diesel::joinable!(graphs -> nodes (entry_node));
diesel::joinable!(graphs -> taint_sources (source));
diesel::joinable!(reachable_nodes -> graphs (graph));
diesel::joinable!(reachable_nodes -> nodes (node));
diesel::joinable!(sink_filters -> nodes (node));

diesel::allow_tables_to_appear_in_same_query!(
    _hidden_analyzed_methods,
    _hidden_graphs,
    analyzed_methods,
    edges,
    external_call_sinks,
    external_field_sinks,
    graph_metadata,
    graphs,
    instruction_sinks,
    nodes,
    reachable_nodes,
    run_info,
    sink_filters,
    source_calls,
    source_fields,
    source_params,
    taint_sources,
);
