// @generated automatically by Diesel CLI.

diesel::table! {
    _hidden_analyzed_methods (analyzed_method) {
        analyzed_method -> Integer,
    }
}

diesel::table! {
    _hidden_routes (route) {
        route -> Integer,
    }
}

diesel::table! {
    analyzed_methods (id) {
        id -> Integer,
        method -> Integer,
        direct -> Bool,
        status -> Text,
        error -> Nullable<Text>,
        route_count -> Integer,
    }
}

diesel::table! {
    call_graph_chains (analyzed_method, chain, idx) {
        analyzed_method -> Integer,
        chain -> Integer,
        idx -> Integer,
        method -> Integer,
    }
}

diesel::table! {
    external_sinks (id) {
        id -> Integer,
        class -> Text,
        name -> Text,
        signature -> Text,
    }
}

diesel::table! {
    field_sinks (id) {
        id -> Integer,
        class -> Text,
        name -> Text,
    }
}

diesel::table! {
    instruction_sinks (id) {
        id -> Integer,
        instruction -> Text,
    }
}

diesel::table! {
    routes (id) {
        id -> Integer,
        source -> Integer,
        incomplete -> Bool,
        phi_count -> Integer,
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
        route -> Integer,
        idx -> Integer,
        kind -> Text,
        class -> Nullable<Text>,
        name -> Nullable<Text>,
        args -> Nullable<Text>,
        ret -> Nullable<Text>,
    }
}

diesel::table! {
    sinks (route, idx) {
        route -> Integer,
        idx -> Integer,
        location -> Integer,
        kind -> Text,
        sink_id -> Nullable<Integer>,
        method_id -> Nullable<Integer>,
    }
}

diesel::table! {
    source_calls (source) {
        source -> Integer,
        class -> Nullable<Text>,
        name -> Text,
        args -> Text,
        ret -> Nullable<Text>,
    }
}

diesel::table! {
    source_fields (source) {
        source -> Integer,
        class -> Text,
        name -> Text,
    }
}

diesel::table! {
    source_params (source) {
        source -> Integer,
        register -> Integer,
    }
}

diesel::table! {
    taint_sources (id) {
        id -> Integer,
        analyzed_method -> Integer,
        kind -> Text,
    }
}

diesel::joinable!(_hidden_analyzed_methods -> analyzed_methods (analyzed_method));
diesel::joinable!(_hidden_routes -> routes (route));
diesel::joinable!(call_graph_chains -> analyzed_methods (analyzed_method));
diesel::joinable!(routes -> taint_sources (source));
diesel::joinable!(sinks -> routes (route));
diesel::joinable!(source_calls -> taint_sources (source));
diesel::joinable!(source_fields -> taint_sources (source));
diesel::joinable!(source_params -> taint_sources (source));
diesel::joinable!(taint_sources -> analyzed_methods (analyzed_method));

diesel::allow_tables_to_appear_in_same_query!(
    _hidden_analyzed_methods,
    _hidden_routes,
    analyzed_methods,
    call_graph_chains,
    external_sinks,
    field_sinks,
    instruction_sinks,
    routes,
    run_info,
    sink_filters,
    sinks,
    source_calls,
    source_fields,
    source_params,
    taint_sources,
);
