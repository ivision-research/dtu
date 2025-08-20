// @generated automatically by Diesel CLI.

diesel::table! {
    activities (id) {
        id -> Integer,
        class_name -> Text,
        permission -> Nullable<Text>,
        exported -> Bool,
        enabled -> Bool,
        pkg -> Text,
        apk_id -> Integer,
    }
}

diesel::table! {
    activity_diffs (id) {
        id -> Integer,
        activity -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
        exported_matches_diff -> Bool,
        permission_matches_diff -> Bool,
        diff_permission -> Nullable<Text>,
    }
}

diesel::table! {
    apk_diffs (id) {
        id -> Integer,
        apk -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
    }
}

diesel::table! {
    apk_permissions (id) {
        id -> Integer,
        name -> Text,
        apk_id -> Integer,
    }
}

diesel::table! {
    apks (id) {
        id -> Integer,
        app_name -> Text,
        name -> Text,
        is_debuggable -> Bool,
        is_priv -> Bool,
        device_path -> Text,
    }
}

diesel::table! {
    device_properties (id) {
        id -> Integer,
        name -> Text,
        value -> Text,
    }
}

diesel::table! {
    diff_sources (id) {
        id -> Integer,
        name -> Text,
    }
}

diesel::table! {
    fuzz_results (id) {
        id -> Integer,
        service_name -> Text,
        method_name -> Text,
        exception_thrown -> Bool,
        security_exception_thrown -> Bool,
    }
}

diesel::table! {
    permission_diffs (id) {
        id -> Integer,
        permission -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
        protection_level_matches_diff -> Bool,
        diff_protection_level -> Nullable<Text>,
    }
}

diesel::table! {
    permissions (id) {
        id -> Integer,
        name -> Text,
        protection_level -> Text,
        source_apk_id -> Integer,
    }
}

diesel::table! {
    protected_broadcasts (id) {
        id -> Integer,
        name -> Text,
    }
}

diesel::table! {
    provider_diffs (id) {
        id -> Integer,
        provider -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
        exported_matches_diff -> Bool,
        permission_matches_diff -> Bool,
        diff_permission -> Nullable<Text>,
        write_permission_matches_diff -> Bool,
        diff_write_permission -> Nullable<Text>,
        read_permission_matches_diff -> Bool,
        diff_read_permission -> Nullable<Text>,
    }
}

diesel::table! {
    providers (id) {
        id -> Integer,
        name -> Text,
        authorities -> Text,
        permission -> Nullable<Text>,
        grant_uri_permissions -> Bool,
        read_permission -> Nullable<Text>,
        write_permission -> Nullable<Text>,
        exported -> Bool,
        enabled -> Bool,
        apk_id -> Integer,
    }
}

diesel::table! {
    receiver_diffs (id) {
        id -> Integer,
        receiver -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
        exported_matches_diff -> Bool,
        permission_matches_diff -> Bool,
        diff_permission -> Nullable<Text>,
    }
}

diesel::table! {
    receivers (id) {
        id -> Integer,
        class_name -> Text,
        permission -> Nullable<Text>,
        exported -> Bool,
        enabled -> Bool,
        pkg -> Text,
        apk_id -> Integer,
    }
}

diesel::table! {
    service_diffs (id) {
        id -> Integer,
        service -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
        exported_matches_diff -> Bool,
        permission_matches_diff -> Bool,
        diff_permission -> Nullable<Text>,
    }
}

diesel::table! {
    services (id) {
        id -> Integer,
        class_name -> Text,
        permission -> Nullable<Text>,
        exported -> Bool,
        enabled -> Bool,
        pkg -> Text,
        apk_id -> Integer,
        returns_binder -> Integer,
    }
}

diesel::table! {
    system_service_diffs (id) {
        id -> Integer,
        system_service -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
    }
}

diesel::table! {
    system_service_impls (id) {
        id -> Integer,
        system_service_id -> Integer,
        source -> Text,
        class_name -> Text,
    }
}

diesel::table! {
    system_service_method_diffs (id) {
        id -> Integer,
        method -> Integer,
        diff_source -> Integer,
        exists_in_diff -> Bool,
        hash_matches_diff -> Integer,
    }
}

diesel::table! {
    system_service_methods (id) {
        id -> Integer,
        system_service_id -> Integer,
        transaction_id -> Integer,
        name -> Text,
        signature -> Nullable<Text>,
        return_type -> Nullable<Text>,
        smalisa_hash -> Nullable<Text>,
    }
}

diesel::table! {
    system_services (id) {
        id -> Integer,
        name -> Text,
        iface -> Nullable<Text>,
        can_get_binder -> Integer,
    }
}

diesel::table! {
    unprotected_broadcasts (id) {
        id -> Integer,
        name -> Text,
        diff_source -> Integer,
    }
}

diesel::joinable!(activities -> apks (apk_id));
diesel::joinable!(activity_diffs -> activities (activity));
diesel::joinable!(activity_diffs -> diff_sources (diff_source));
diesel::joinable!(apk_diffs -> apks (apk));
diesel::joinable!(apk_diffs -> diff_sources (diff_source));
diesel::joinable!(apk_permissions -> apks (apk_id));
diesel::joinable!(permission_diffs -> diff_sources (diff_source));
diesel::joinable!(permission_diffs -> permissions (permission));
diesel::joinable!(permissions -> apks (source_apk_id));
diesel::joinable!(provider_diffs -> diff_sources (diff_source));
diesel::joinable!(provider_diffs -> providers (provider));
diesel::joinable!(providers -> apks (apk_id));
diesel::joinable!(receiver_diffs -> diff_sources (diff_source));
diesel::joinable!(receiver_diffs -> receivers (receiver));
diesel::joinable!(receivers -> apks (apk_id));
diesel::joinable!(service_diffs -> diff_sources (diff_source));
diesel::joinable!(service_diffs -> services (service));
diesel::joinable!(services -> apks (apk_id));
diesel::joinable!(system_service_diffs -> diff_sources (diff_source));
diesel::joinable!(system_service_diffs -> system_services (system_service));
diesel::joinable!(system_service_impls -> system_services (system_service_id));
diesel::joinable!(system_service_method_diffs -> diff_sources (diff_source));
diesel::joinable!(system_service_method_diffs -> system_service_methods (method));
diesel::joinable!(system_service_methods -> system_services (system_service_id));
diesel::joinable!(unprotected_broadcasts -> diff_sources (diff_source));

diesel::allow_tables_to_appear_in_same_query!(
    activities,
    activity_diffs,
    apk_diffs,
    apk_permissions,
    apks,
    device_properties,
    diff_sources,
    fuzz_results,
    permission_diffs,
    permissions,
    protected_broadcasts,
    provider_diffs,
    providers,
    receiver_diffs,
    receivers,
    service_diffs,
    services,
    system_service_diffs,
    system_service_impls,
    system_service_method_diffs,
    system_service_methods,
    system_services,
    unprotected_broadcasts,
);
