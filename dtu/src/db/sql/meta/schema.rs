// @generated automatically by Diesel CLI.

diesel::table! {
    app_activities (id) {
        id -> Integer,
        name -> Text,
        button_android_id -> Text,
        button_text -> Text,
        status -> Integer,
    }
}

diesel::table! {
    app_permissions (id) {
        id -> Integer,
        permission -> Text,
        usable -> Bool,
    }
}

diesel::table! {
    decompile_status (id) {
        id -> Integer,
        device_path -> Text,
        host_path -> Nullable<Text>,
        decompiled -> Bool,
        decompile_attempts -> Integer,
    }
}

diesel::table! {
    key_values (id) {
        id -> Integer,
        key -> Text,
        value -> Text,
    }
}

diesel::table! {
    progress (step) {
        step -> Integer,
        completed -> Bool,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    app_activities,
    app_permissions,
    decompile_status,
    key_values,
    progress,
);
