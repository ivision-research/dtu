// @generated automatically by Diesel CLI.

diesel::table! {
    _load_status (rowid) {
        rowid -> Integer,
        source -> Integer,
        kind -> Integer,
    }
}

diesel::table! {
    calls (rowid) {
        rowid -> Integer,
        caller -> Integer,
        callee -> Integer,
        source -> Integer,
    }
}

diesel::table! {
    classes (id) {
        id -> Integer,
        name -> Text,
        access_flags -> BigInt,
        source -> Integer,
    }
}

diesel::table! {
    interfaces (rowid) {
        rowid -> Integer,
        interface -> Integer,
        class -> Integer,
        source -> Integer,
    }
}

diesel::table! {
    method_strings (id) {
        id -> Integer,
        string -> Text,
        method -> Integer,
        source -> Integer,
    }
}

diesel::table! {
    methods (id) {
        id -> Integer,
        class -> Integer,
        name -> Text,
        args -> Text,
        ret -> Text,
        access_flags -> BigInt,
        source -> Integer,
    }
}

diesel::table! {
    sources (id) {
        id -> Integer,
        name -> Text,
    }
}

diesel::table! {
    supers (rowid) {
        rowid -> Integer,
        parent -> Integer,
        child -> Integer,
        source -> Integer,
    }
}

diesel::joinable!(_load_status -> sources (source));
diesel::joinable!(calls -> sources (source));
diesel::joinable!(classes -> sources (source));
diesel::joinable!(interfaces -> sources (source));
diesel::joinable!(method_strings -> methods (id));
diesel::joinable!(method_strings -> sources (source));
diesel::joinable!(methods -> classes (class));
diesel::joinable!(methods -> sources (source));
diesel::joinable!(supers -> sources (source));

diesel::allow_tables_to_appear_in_same_query!(
    _load_status,
    calls,
    classes,
    interfaces,
    method_strings,
    methods,
    sources,
    supers,
);
