use diesel::delete;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations};

pub const EMULATOR_DIFF_SOURCE: &'static str = "emulator";

use crate::Context;

use super::common::*;
use super::models::*;
use super::schema;

macro_rules! def_diff_item {
    (
        $ins_one:ident,
        $ins_type:ty,
        $get_all_by_diff_name:ident,
        $get_all_by_diff_id:ident,
        $get_type:ty
    ) => {
        def_insert_one!($ins_one, $ins_type);
        def_get_multi_by!($get_all_by_diff_id, i32, $get_type);
        def_get_multi_by!($get_all_by_diff_name, &str, $get_type);
    };

    (
        $ins_one:ident,
        $ins_type:ty,
        $get_all_by_diff_name:ident,
        $get_all_by_diff_id:ident,
        $get_type:ty,
        $get_by_two_ids:ident
    ) => {
        def_diff_item!(
            $ins_one,
            $ins_type,
            $get_all_by_diff_name,
            $get_all_by_diff_id,
            $get_type
        );
        fn $get_by_two_ids(&self, owner_id: i32, diff_id: i32) -> Result<Vec<$get_type>>;
    };
}

pub trait Database: Sync + Send {
    fn wipe(&self) -> Result<()>;
    def_standard_crud!(
        add_permission,
        add_permissions,
        InsertPermission,
        get_permissions,
        get_permission_by_id,
        update_permission,
        Permission,
        delete_permission_by_id
    );

    def_standard_crud!(
        add_provider,
        add_providers,
        InsertProvider,
        get_providers,
        get_provider_by_id,
        update_provider,
        Provider,
        delete_provider_by_id
    );

    def_standard_crud!(
        add_service,
        add_services,
        InsertService,
        get_services,
        get_service_by_id,
        update_service,
        Service,
        delete_service_by_id
    );

    def_standard_crud!(
        add_receiver,
        add_receivers,
        InsertReceiver,
        get_receivers,
        get_receiver_by_id,
        update_receiver,
        Receiver,
        delete_receiver_by_id
    );

    def_standard_crud!(
        add_activity,
        add_activities,
        InsertActivity,
        get_activities,
        get_activity_by_id,
        update_activity,
        Activity,
        delete_activity_by_id
    );

    def_standard_crud!(
        add_system_service,
        add_system_services,
        InsertSystemService,
        get_system_services,
        get_system_service_by_id,
        update_system_service,
        SystemService,
        delete_system_service_by_id
    );

    def_delete_by!(delete_system_service_methods_by_service_id, i32);
    def_get_multi_by!(get_system_services_name_like, &str, SystemService);
    def_get_one_by!(get_system_service_by_name, &str, SystemService);
    def_get_multi!(get_system_service_methods, SystemServiceMethod);

    def_standard_crud!(
        add_device_property,
        add_device_properties,
        InsertDeviceProperty,
        get_device_properties,
        get_device_property_by_id,
        update_device_property,
        DeviceProperty,
        delete_device_property_by_id
    );

    def_get_one_by!(get_device_property_by_name, &str, DeviceProperty);
    def_get_multi_by!(get_device_properties_like, &str, DeviceProperty);

    def_insert_one!(add_apk_permission, InsertApkPermission);
    def_insert_multi!(add_apk_permissions, InsertApkPermission);
    fn get_permissions_for_apk(&self, apk: &Apk) -> Result<Vec<ApkPermission>>;
    fn get_all_apks_with_permsissions(&self) -> Result<Vec<ApkWithPermissions>>;

    def_standard_crud!(
        add_apk,
        add_apks,
        InsertApk,
        get_apks,
        get_apk_by_id,
        update_apk,
        Apk,
        delete_apk_by_id
    );

    def_get_multi!(get_debuggable_apks, Apk);
    def_get_one_by!(get_apk_by_app_name, &str, Apk);
    def_get_one_by!(get_apk_by_apk_name, &str, Apk);
    def_get_one_by!(get_apk_by_device_path, &str, Apk);
    def_get_multi!(get_normal_permissions, Permission);
    def_get_one_by!(get_permission_by_apk, i32, Permission);
    def_get_one_by!(get_permission_by_name, &str, Permission);
    def_get_multi_by!(get_permissions_by_name_like, &str, Permission);
    def_insert_one!(add_system_service_impl, InsertSystemServiceImpl);
    def_get_multi_by!(get_system_service_impls, i32, SystemServiceImpl);
    def_delete_by!(delete_system_service_impl_by_service_id, i32);
    def_insert_one!(add_system_service_method, InsertSystemServiceMethod);
    def_update_one!(update_system_service_method, SystemServiceMethod);

    def_get_multi_by!(
        get_system_service_methods_by_service_id,
        i32,
        SystemServiceMethod
    );

    def_get_one_by!(get_provider_containing_authority, &str, Provider);
    def_get_multi_by!(get_receivers_by_apk_id, i32, Receiver);
    def_get_multi_by!(get_services_by_apk_id, i32, Service);
    def_get_multi_by!(get_activities_by_apk_id, i32, Activity);
    def_get_multi_by!(get_providers_by_apk_id, i32, Provider);

    def_insert_one!(add_diff_source, InsertDiffSource);
    def_get_multi!(get_diff_sources, DiffSource);
    def_get_one_by!(get_diff_source_by_name, &str, DiffSource);
    def_delete_by!(delete_diff_source_by_id, i32);
    def_delete_by!(delete_diff_source_by_name, &str);

    def_diff_item!(
        add_permission_diff,
        InsertPermissionDiff,
        get_permission_diffs_by_diff_name,
        get_permission_diffs_by_diff_id,
        DiffedPermission
    );

    def_diff_item!(
        add_apk_diff,
        InsertApkDiff,
        get_apk_diffs_by_diff_name,
        get_apk_diffs_by_diff_id,
        DiffedApk
    );

    def_diff_item!(
        add_system_service_diff,
        InsertSystemServiceDiff,
        get_system_service_diffs_by_diff_name,
        get_system_service_diffs_by_diff_id,
        DiffedSystemService
    );
    def_delete_by!(delete_system_service_diff_by_service_id, i32);

    def_diff_item!(
        add_system_service_method_diff,
        InsertSystemServiceMethodDiff,
        get_system_service_method_diffs_by_diff_name,
        get_system_service_method_diffs_by_diff_id,
        DiffedSystemServiceMethod,
        get_system_service_method_diffs_for_service
    );

    def_diff_item!(
        add_service_diff,
        InsertServiceDiff,
        get_service_diffs_by_diff_name,
        get_service_diffs_by_diff_id,
        DiffedService,
        get_service_diffs_for_apk
    );

    def_diff_item!(
        add_provider_diff,
        InsertProviderDiff,
        get_provider_diffs_by_diff_name,
        get_provider_diffs_by_diff_id,
        DiffedProvider,
        get_provider_diffs_for_apk
    );

    def_diff_item!(
        add_activity_diff,
        InsertActivityDiff,
        get_activity_diffs_by_diff_name,
        get_activity_diffs_by_diff_id,
        DiffedActivity,
        get_activity_diffs_for_apk
    );

    def_diff_item!(
        add_receiver_diff,
        InsertReceiverDiff,
        get_receiver_diffs_by_diff_name,
        get_receiver_diffs_by_diff_id,
        DiffedReceiver,
        get_receiver_diffs_for_apk
    );
    def_standard_crud!(
        add_fuzz_result,
        add_fuzz_results,
        InsertFuzzResult,
        get_fuzz_results,
        get_fuzz_result_by_id,
        update_fuzz_result,
        FuzzResult,
        delete_fuzz_result_by_id
    );
    def_get_multi_by!(get_endpoints_by_security, bool, FuzzResult);
}

#[derive(Clone)]
pub struct DeviceSqliteDatabase {
    db_thread: DBThread,
}

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/device_migrations/");

#[cfg(test)]
const TEST_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/test_device_migrations/");

pub static DEVICE_DATABASE_FILE_NAME: &'static str = "device.db";

impl DeviceSqliteDatabase {
    pub fn new(ctx: &dyn Context) -> Result<Self> {
        Ok(Self {
            db_thread: DBThread::new(
                ctx,
                DEVICE_DATABASE_FILE_NAME,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        })
    }

    pub fn new_from_path<S: AsRef<str> + ?Sized>(path: &S) -> Result<Self> {
        Ok(Self {
            db_thread: DBThread::new_from_path(
                path,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        })
    }

    #[cfg(test)]
    fn new_from_url(url: &String) -> Result<Self> {
        Ok(Self {
            db_thread: DBThread::new_from_url(
                url,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        })
    }

    #[inline]
    fn with_connection<F, R>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce(&mut SqliteConnection) -> R + Send,
    {
        self.db_thread.with_connection(f)
    }
}

macro_rules! impl_diff_item {
    (
        $ins_one:ident,
        $ins_type:ty,
        $get_all_by_diff_name:ident,
        $get_all_by_diff_id:ident,
        $left_type:ident,
        $right_type:ident,
        $get_type:ident,
        $table:ident,
        $diff_table:ident
    ) => {
        impl_insert_one!($ins_one, $ins_type, $diff_table);

        fn $get_all_by_diff_name(&self, name: &str) -> Result<Vec<$get_type>> {
            let ds = self.get_diff_source_by_name(name)?;
            self.$get_all_by_diff_id(ds.id)
        }

        fn $get_all_by_diff_id(&self, id: i32) -> Result<Vec<$get_type>> {
            self.with_connection(|conn| {
                let __query = super::schema::$table::table
                    .inner_join(super::schema::$diff_table::table)
                    .filter(super::schema::$diff_table::dsl::diff_source.eq(id));
                #[cfg(feature = "trace_db")]
                ::log::trace!(
                    "{}",
                    diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query)
                );
                let res: ::std::vec::Vec<($left_type, $right_type)> = __query.load(conn)?;
                Ok(res
                    .into_iter()
                    .map($get_type::from)
                    .collect::<Vec<$get_type>>())
            })
        }
    };

    (
        $ins_one:ident,
        $ins_type:ty,
        $get_all_by_diff_name:ident,
        $get_all_by_diff_id:ident,
        $left_type:ident,
        $right_type:ident,
        $get_type:ident,
        $table:ident,
        $diff_table:ident,
        $get_all_by_two_ids:ident,
        $($other_filter:tt)+
    ) => {
        impl_diff_item!(
            $ins_one,
            $ins_type,
            $get_all_by_diff_name,
            $get_all_by_diff_id,
            $left_type,
            $right_type,
            $get_type,
            $table,
            $diff_table
        );

        fn $get_all_by_two_ids(&self, owner_id: i32, diff_id: i32) -> Result<Vec<$get_type>> {
                        self.with_connection(|conn| {
                let __query = super::schema::$table::table
                    .inner_join(super::schema::$diff_table::table)
                    .filter(super::schema::$diff_table::dsl::diff_source.eq(diff_id))
                    .filter(super::schema::$table::dsl::$($other_filter)+(owner_id));
                #[cfg(feature = "trace_db")]
                ::log::trace!(
                    "{}",
                    diesel::debug_query::<::diesel::sqlite::Sqlite, _>(&__query)
                );
                let res: ::std::vec::Vec<($left_type, $right_type)> = __query.load(conn)?;
                Ok(res
                    .into_iter()
                    .map($get_type::from)
                    .collect::<Vec<$get_type>>())
            })
        }
    };
}

impl Database for DeviceSqliteDatabase {
    fn wipe(&self) -> Result<()> {
        log::debug!("wiping the database");
        self.with_connection(|conn| {
            conn.transaction(|txn| {
                delete(schema::apk_permissions::dsl::apk_permissions).execute(txn)?;
                delete(schema::apk_diffs::dsl::apk_diffs).execute(txn)?;
                delete(schema::device_properties::dsl::device_properties).execute(txn)?;
                delete(schema::system_services::dsl::system_services).execute(txn)?;
                delete(schema::apks::dsl::apks).execute(txn)?;
                delete(schema::protected_broadcasts::dsl::protected_broadcasts).execute(txn)?;
                delete(schema::fuzz_results::dsl::fuzz_results).execute(txn)?;
                delete(schema::diff_sources::dsl::diff_sources)
                    .filter(schema::diff_sources::dsl::name.is_not(EMULATOR_DIFF_SOURCE))
                    .execute(txn)?;
                Ok(())
            })
        })
    }

    impl_get_multi_by!(
        get_system_services_name_like,
        &str,
        SystemService,
        system_services,
        name.like
    );

    impl_standard_crud!(
        device_properties,
        add_device_property,
        add_device_properties,
        InsertDeviceProperty,
        get_device_properties,
        get_device_property_by_id,
        update_device_property,
        DeviceProperty,
        delete_device_property_by_id
    );

    impl_get_one_by!(
        get_device_property_by_name,
        &str,
        DeviceProperty,
        device_properties,
        name.eq
    );

    impl_get_multi_by!(
        get_device_properties_like,
        &str,
        DeviceProperty,
        device_properties,
        name.like
    );

    impl_insert_one!(add_apk_permission, InsertApkPermission, apk_permissions);
    impl_insert_multi!(add_apk_permissions, InsertApkPermission, apk_permissions);

    fn get_permissions_for_apk(&self, apk: &Apk) -> Result<Vec<ApkPermission>> {
        self.with_connection(|conn| {
            let perms = ApkPermission::belonging_to(apk)
                .select(ApkPermission::as_select())
                .load(conn)?;
            Ok(perms)
        })
    }
    fn get_all_apks_with_permsissions(&self) -> Result<Vec<ApkWithPermissions>> {
        let apks = self.get_apks()?;
        self.with_connection(|conn| {
            let perms = ApkPermission::belonging_to(&apks)
                .select(ApkPermission::as_select())
                .load(conn)?;

            Ok(perms
                .grouped_by(&apks)
                .into_iter()
                .zip(apks)
                .map(|(perms, apk)| ApkWithPermissions {
                    apk,
                    permissions: perms.into_iter().map(|it| it.name).collect::<Vec<String>>(),
                })
                .collect::<Vec<ApkWithPermissions>>())
        })
    }

    impl_standard_crud!(
        activities,
        add_activity,
        add_activities,
        InsertActivity,
        get_activities,
        get_activity_by_id,
        update_activity,
        Activity,
        delete_activity_by_id
    );

    impl_standard_crud!(
        receivers,
        add_receiver,
        add_receivers,
        InsertReceiver,
        get_receivers,
        get_receiver_by_id,
        update_receiver,
        Receiver,
        delete_receiver_by_id
    );

    impl_standard_crud!(
        services,
        add_service,
        add_services,
        InsertService,
        get_services,
        get_service_by_id,
        update_service,
        Service,
        delete_service_by_id
    );

    impl_standard_crud!(
        permissions,
        add_permission,
        add_permissions,
        InsertPermission,
        get_permissions,
        get_permission_by_id,
        update_permission,
        Permission,
        delete_permission_by_id
    );

    impl_standard_crud!(
        providers,
        add_provider,
        add_providers,
        InsertProvider,
        get_providers,
        get_provider_by_id,
        update_provider,
        Provider,
        delete_provider_by_id
    );

    impl_standard_crud!(
        apks,
        add_apk,
        add_apks,
        InsertApk,
        get_apks,
        get_apk_by_id,
        update_apk,
        Apk,
        delete_apk_by_id
    );
    impl_get_multi!(get_debuggable_apks, Apk, apks, is_debuggable.eq(true));
    impl_get_one_by!(get_apk_by_app_name, &str, Apk, apks, app_name.eq);
    impl_get_one_by!(get_apk_by_apk_name, &str, Apk, apks, name.eq);
    impl_get_one_by!(get_apk_by_device_path, &str, Apk, apks, device_path.eq);

    impl_get_all!(
        get_system_service_methods,
        SystemServiceMethod,
        system_service_methods
    );

    impl_get_multi!(
        get_normal_permissions,
        Permission,
        permissions,
        protection_level.like("%normal%")
    );

    impl_get_one_by!(
        get_permission_by_apk,
        i32,
        Permission,
        permissions,
        source_apk_id.eq
    );

    impl_get_multi_by!(
        get_permissions_by_name_like,
        &str,
        Permission,
        permissions,
        name.like
    );

    impl_get_one_by!(
        get_permission_by_name,
        &str,
        Permission,
        permissions,
        name.eq
    );

    impl_standard_crud!(
        system_services,
        add_system_service,
        add_system_services,
        InsertSystemService,
        get_system_services,
        get_system_service_by_id,
        update_system_service,
        SystemService,
        delete_system_service_by_id
    );

    impl_get_one_by!(
        get_system_service_by_name,
        &str,
        SystemService,
        system_services,
        name.eq
    );

    impl_insert_one!(add_diff_source, InsertDiffSource, diff_sources);

    impl_insert_one!(
        add_system_service_impl,
        InsertSystemServiceImpl,
        system_service_impls
    );

    impl_insert_one!(
        add_system_service_method,
        InsertSystemServiceMethod,
        system_service_methods
    );

    impl_get_multi_by!(
        get_system_service_impls,
        i32,
        SystemServiceImpl,
        system_service_impls,
        system_service_id.eq
    );

    impl_get_multi_by!(
        get_system_service_methods_by_service_id,
        i32,
        SystemServiceMethod,
        system_service_methods,
        system_service_id.eq
    );
    impl_update_one!(
        update_system_service_method,
        SystemServiceMethod,
        system_service_methods
    );

    fn get_provider_containing_authority(&self, sel: &str) -> Result<Provider> {
        use super::schema::providers::dsl::*;
        let like_middle = format!("%:{}:%", sel);
        let like_left = format!("{}:%", sel);
        let like_right = format!("%:{}", sel);
        self.with_connection(|c| {
            Ok(providers
                .filter(
                    authorities
                        .eq(sel)
                        .or(authorities.like(like_middle))
                        .or(authorities.like(like_left))
                        .or(authorities.like(like_right)),
                )
                .get_result(c)?)
        })
    }

    impl_get_multi_by!(get_receivers_by_apk_id, i32, Receiver, receivers, apk_id.eq);
    impl_get_multi_by!(get_services_by_apk_id, i32, Service, services, apk_id.eq);
    impl_get_multi_by!(
        get_activities_by_apk_id,
        i32,
        Activity,
        activities,
        apk_id.eq
    );
    impl_get_multi_by!(get_providers_by_apk_id, i32, Provider, providers, apk_id.eq);

    impl_get_all!(get_diff_sources, DiffSource, diff_sources);
    impl_get_one_by!(
        get_diff_source_by_name,
        &str,
        DiffSource,
        diff_sources,
        name.eq
    );
    impl_delete_by!(
        delete_system_service_diff_by_service_id,
        i32,
        system_service_diffs,
        system_service.eq
    );

    impl_delete_by!(
        delete_system_service_methods_by_service_id,
        i32,
        system_service_methods,
        system_service_id.eq
    );

    impl_delete_by!(
        delete_system_service_impl_by_service_id,
        i32,
        system_service_impls,
        system_service_id.eq
    );
    impl_delete_by!(delete_diff_source_by_id, i32, diff_sources, id.eq);
    impl_delete_by!(delete_diff_source_by_name, &str, diff_sources, name.eq);

    impl_diff_item!(
        add_permission_diff,
        InsertPermissionDiff,
        get_permission_diffs_by_diff_name,
        get_permission_diffs_by_diff_id,
        Permission,
        PermissionDiff,
        DiffedPermission,
        permissions,
        permission_diffs
    );

    impl_diff_item!(
        add_apk_diff,
        InsertApkDiff,
        get_apk_diffs_by_diff_name,
        get_apk_diffs_by_diff_id,
        Apk,
        ApkDiff,
        DiffedApk,
        apks,
        apk_diffs
    );

    impl_diff_item!(
        add_system_service_diff,
        InsertSystemServiceDiff,
        get_system_service_diffs_by_diff_name,
        get_system_service_diffs_by_diff_id,
        SystemService,
        SystemServiceDiff,
        DiffedSystemService,
        system_services,
        system_service_diffs
    );

    impl_diff_item!(
        add_system_service_method_diff,
        InsertSystemServiceMethodDiff,
        get_system_service_method_diffs_by_diff_name,
        get_system_service_method_diffs_by_diff_id,
        SystemServiceMethod,
        SystemServiceMethodDiff,
        DiffedSystemServiceMethod,
        system_service_methods,
        system_service_method_diffs,
        get_system_service_method_diffs_for_service,
        system_service_id.eq
    );

    impl_diff_item!(
        add_service_diff,
        InsertServiceDiff,
        get_service_diffs_by_diff_name,
        get_service_diffs_by_diff_id,
        Service,
        ServiceDiff,
        DiffedService,
        services,
        service_diffs,
        get_service_diffs_for_apk,
        apk_id.eq
    );

    impl_diff_item!(
        add_provider_diff,
        InsertProviderDiff,
        get_provider_diffs_by_diff_name,
        get_provider_diffs_by_diff_id,
        Provider,
        ProviderDiff,
        DiffedProvider,
        providers,
        provider_diffs,
        get_provider_diffs_for_apk,
        apk_id.eq
    );

    impl_diff_item!(
        add_activity_diff,
        InsertActivityDiff,
        get_activity_diffs_by_diff_name,
        get_activity_diffs_by_diff_id,
        Activity,
        ActivityDiff,
        DiffedActivity,
        activities,
        activity_diffs,
        get_activity_diffs_for_apk,
        apk_id.eq
    );

    impl_diff_item!(
        add_receiver_diff,
        InsertReceiverDiff,
        get_receiver_diffs_by_diff_name,
        get_receiver_diffs_by_diff_id,
        Receiver,
        ReceiverDiff,
        DiffedReceiver,
        receivers,
        receiver_diffs,
        get_receiver_diffs_for_apk,
        apk_id.eq
    );
    impl_standard_crud!(
        fuzz_results,
        add_fuzz_result,
        add_fuzz_results,
        InsertFuzzResult,
        get_fuzz_results,
        get_fuzz_result_by_id,
        update_fuzz_result,
        FuzzResult,
        delete_fuzz_result_by_id
    );
    impl_get_multi_by!(
        get_endpoints_by_security,
        bool,
        FuzzResult,
        fuzz_results,
        security_exception_thrown.eq
    );
}

impl From<ConnectionError> for Error {
    fn from(value: ConnectionError) -> Self {
        Self::ConnectionError(value)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rstest::*;
    use std::panic;
    use std::panic::AssertUnwindSafe;

    use super::super::common::cleanup_database;
    use crate::testing::{tmp_context, TestContext};
    use crate::utils::{ensure_dir_exists, DevicePath};
    use crate::UnknownBool;

    fn get_db_url(context: &dyn Context) -> String {
        let dir = context.get_sqlite_dir().expect("failed to get sqlite dir");
        ensure_dir_exists(&dir).expect("failed to make dir");
        format!(
            "sqlite://{}",
            dir.join(DEVICE_DATABASE_FILE_NAME).to_string_lossy()
        )
    }

    macro_rules! assert_err {
        ($value:expr, $($err:tt)+) => {{
            let res = $value;
            match res {
                Err(Error::$($err)*) => {},
                _ => panic!(
                    "expected error Error::{} got {:?}",
                    concat!($(stringify!($err)),*),
                    res
                ),
            }
        }};
    }

    fn db_test(context: &dyn Context, func: impl FnOnce(DeviceSqliteDatabase)) {
        let url = get_db_url(&context);
        let db = DeviceSqliteDatabase::new_from_url(&url).expect("failed to get database");
        let res = panic::catch_unwind(AssertUnwindSafe(|| func(db)));
        cleanup_database(&url);
        match res {
            Err(e) => panic::resume_unwind(e),
            _ => {}
        }
    }

    #[rstest]
    fn test_system_service_crud(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let name = "insertable";
            let ins = InsertSystemService::new(name, UnknownBool::False);
            let id = db.add_system_service(&ins).expect("failed to add service");
            let mut ent = db
                .get_system_service_by_id(id)
                .expect("failed to get service");

            ent.name = String::from("updated");
            ent.can_get_binder = UnknownBool::True;

            db.update_system_service(&ent)
                .expect("failed to update system service");

            let updated = db
                .get_system_service_by_id(ent.id)
                .expect("failed to get by id");

            assert_eq!(updated, ent, "update failed");

            db.delete_system_service_by_id(ent.id)
                .expect("failed to delete service by id");
        })
    }

    #[rstest]
    fn test_get_providers_containing_authority(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let exact = db
                .get_provider_containing_authority("exact.authority")
                .unwrap();
            assert_eq!(exact.authorities, "exact.authority");
            let left = db
                .get_provider_containing_authority("left.authority")
                .unwrap();
            assert_eq!(left.authorities, "left.authority:not.left.authority");
            let right = db
                .get_provider_containing_authority("right.authority")
                .unwrap();
            assert_eq!(right.authorities, "not.right.authority:right.authority");
            let middle = db
                .get_provider_containing_authority("middle.authority")
                .unwrap();
            assert_eq!(
                middle.authorities,
                "left.not.middle.authority:middle.authority:right.not.middle.authority"
            );
        });
    }

    #[rstest]
    fn test_get_system_services_name_like(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let res = db
                .get_system_services_name_like("test%")
                .expect("shouldn't have failed");
            assert_ne!(res.len(), 0, "should have found something");
        });
    }

    #[rstest]
    fn test_get_system_services(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            assert_ne!(
                db.get_system_services()
                    .expect("should not have errored")
                    .len(),
                0,
                "should have returned multiple rows"
            );

            assert_err!(db.get_system_service_by_name("DOESNTEXIST"), NotFound);

            assert!(
                db.get_system_service_by_name("test_can").is_ok(),
                "should have retrieved a valid system service entry"
            );
        });
    }

    #[rstest]
    fn test_apk_crud(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let ins = InsertApk::new(
                "new.apk",
                "NewApk.apk",
                false,
                true,
                DevicePath::new("/system/priv-app/NewApk.apk"),
            );
            let id = db.add_apk(&ins).expect("failed to add apk");
            let mut ent = db.get_apk_by_id(id).expect("failed to get apk");

            ent.name = String::from("updated");
            ent.is_debuggable = true;

            db.update_apk(&ent).expect("failed to update apk");

            let updated = db.get_apk_by_id(ent.id).expect("failed to get by id");

            assert_eq!(updated, ent, "update failed");

            db.delete_apk_by_id(ent.id)
                .expect("failed to delete apk by id");
        })
    }

    #[rstest]
    fn test_get_apks(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            assert_ne!(
                db.get_apks().expect("should not have errored").len(),
                0,
                "should have returned multiple rows"
            );

            assert_err!(db.get_apk_by_app_name("DOESNTEXIST"), NotFound);
            assert_err!(db.get_apk_by_id(0x1337), NotFound);

            assert!(
                db.get_apk_by_app_name("just.an.app").is_ok(),
                "should have retrieved a valid apk entry via name"
            );

            assert!(
                db.get_apk_by_id(0).is_ok(),
                "should have retrieved a valid apk entry via id"
            );
        })
    }
}
