use std::collections::HashMap;

use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations};

pub const EMULATOR_DIFF_SOURCE: &'static str = "emulator";

use super::schema::{system_service_impls, system_services};
use crate::utils::ClassName;
use crate::Context;

use super::common::*;
use super::models::*;

#[derive(Clone)]
pub struct DeviceDatabase {
    db_thread: DBThread,
}

pub type SqlConnection = SqliteConnection;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/device_migrations/");

#[cfg(test)]
const TEST_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/test_device_migrations/");

pub static DEVICE_DATABASE_FILE_NAME: &'static str = "device.db";

impl DeviceDatabase {
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
    pub fn with_connection<F, R>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce(&mut SqlConnection) -> R + Send,
    {
        self.db_thread.with_connection(f)
    }

    #[inline]
    pub fn with_transaction<F, T, E>(&self, f: F) -> std::result::Result<T, E>
    where
        T: Send,
        E: From<diesel::result::Error> + Send,
        F: FnOnce(&mut SqlConnection) -> std::result::Result<T, E> + Send,
    {
        self.db_thread.transaction(f)
    }
}

macro_rules! impl_diff_item {
    (
        $vis:vis
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
        impl_insert_one!($vis $ins_one, $ins_type, $diff_table);

        $vis fn $get_all_by_diff_name(&self, name: &str) -> Result<Vec<$get_type>> {
            let ds = self.get_diff_source_by_name(name)?;
            self.$get_all_by_diff_id(ds.id)
        }

        $vis fn $get_all_by_diff_id(&self, id: i32) -> Result<Vec<$get_type>> {
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
        $vis:vis
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
            $vis
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

        $vis fn $get_all_by_two_ids(&self, owner_id: i32, diff_id: i32) -> Result<Vec<$get_type>> {
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

// A lot of the methods exposed here are historical or are currently being used by the Python API
// and I just haven't had time to rework things. Removing them would be a breaking change, and
// they're not bothering anyone.

impl DeviceDatabase {
    pub fn wipe(&self, ctx: &dyn Context) -> Result<()> {
        log::debug!("wiping the database");
        let path = ctx.get_sqlite_dir()?.join(DEVICE_DATABASE_FILE_NAME);
        std::fs::remove_file(&path)?;
        Ok(())
    }

    impl_get_multi_by!(
        pub
        get_system_services_name_like,
        &str,
        SystemService,
        system_services,
        name.like
    );

    impl_simple_gets!(pub
        protected_broadcasts,
        ProtectedBroadcast,
        get_protected_broadcasts,
        get_protected_broadcast_by_id
    );

    impl_simple_gets!(pub
        device_properties,
        DeviceProperty,
        get_device_properties,
        get_device_property_by_id
    );

    impl_get_one_by!(
        pub
        get_device_property_by_name,
        &str,
        DeviceProperty,
        device_properties,
        name.eq
    );

    impl_get_multi_by!(
        pub
        get_device_properties_like,
        &str,
        DeviceProperty,
        device_properties,
        name.like
    );

    pub fn get_permissions_for_apk(&self, apk: &Apk) -> Result<Vec<ApkPermission>> {
        self.with_connection(|conn| {
            let perms = ApkPermission::belonging_to(apk)
                .select(ApkPermission::as_select())
                .load(conn)?;
            Ok(perms)
        })
    }
    pub fn get_all_apks_with_permsissions(&self) -> Result<Vec<ApkWithPermissions>> {
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

    pub fn get_all_system_service_impls(&self) -> Result<HashMap<String, Vec<SystemServiceImpl>>> {
        self.with_connection(|c| {
            let mut result: HashMap<String, Vec<SystemServiceImpl>> = HashMap::new();

            let rows = system_service_impls::table
                .inner_join(system_services::table)
                .select((
                    system_services::name,
                    system_service_impls::id,
                    system_service_impls::system_service_id,
                    system_service_impls::class_name,
                    system_service_impls::source,
                ))
                .load::<(String, i32, i32, ClassName, String)>(c)?
                .into_iter();

            for (service, id, system_service_id, class_name, source) in rows {
                let impl_ = SystemServiceImpl {
                    id,
                    system_service_id,
                    class_name,
                    source,
                };
                if let Some(into) = result.get_mut(&service) {
                    into.push(impl_);
                } else {
                    let v = vec![impl_];
                    result.insert(service, v);
                }
            }
            Ok(result)
        })
    }

    impl_simple_gets!(pub activities, Activity, get_activities, get_activity_by_id);

    impl_simple_gets!(pub receivers, Receiver, get_receivers, get_receiver_by_id);

    impl_simple_gets!(pub services, Service, get_services, get_service_by_id);

    impl_simple_gets!(
        pub
        permissions,
        Permission,
        get_permissions,
        get_permission_by_id
    );

    impl_simple_gets!(pub providers, Provider, get_providers, get_provider_by_id);

    impl_simple_gets!(pub apks, Apk, get_apks, get_apk_by_id);
    impl_get_multi!(pub get_debuggable_apks, Apk, apks, is_debuggable.eq(true));
    impl_get_one_by!(pub get_apk_by_app_name, &str, Apk, apks, app_name.eq);
    impl_get_one_by!(pub get_apk_by_apk_name, &str, Apk, apks, name.eq);
    impl_get_one_by!(pub get_apk_by_device_path, &str, Apk, apks, device_path.eq);

    impl_get_all!(pub
        get_system_service_methods,
        SystemServiceMethod,
        system_service_methods
    );

    impl_get_multi!(pub
        get_normal_permissions,
        Permission,
        permissions,
        protection_level.like("%normal%")
    );

    impl_get_one_by!(pub
        get_permission_by_apk,
        i32,
        Permission,
        permissions,
        source_apk_id.eq
    );

    impl_get_multi_by!(pub
        get_permissions_by_name_like,
        &str,
        Permission,
        permissions,
        name.like
    );

    impl_get_one_by!(pub
        get_permission_by_name,
        &str,
        Permission,
        permissions,
        name.eq
    );

    impl_simple_gets!(pub
        system_services,
        SystemService,
        get_system_services,
        get_system_service_by_id
    );

    impl_get_one_by!(pub
        get_system_service_by_name,
        &str,
        SystemService,
        system_services,
        name.eq
    );

    impl_get_multi_by!(pub
        get_system_service_impls,
        i32,
        SystemServiceImpl,
        system_service_impls,
        system_service_id.eq
    );

    impl_get_multi_by!(pub
        get_system_service_methods_by_service_id,
        i32,
        SystemServiceMethod,
        system_service_methods,
        system_service_id.eq
    );

    pub fn get_provider_containing_authority(&self, sel: &str) -> Result<Provider> {
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

    impl_get_multi_by!(pub get_receivers_by_apk_id, i32, Receiver, receivers, apk_id.eq);
    impl_get_multi_by!(pub get_services_by_apk_id, i32, Service, services, apk_id.eq);
    impl_get_multi_by!(pub
        get_activities_by_apk_id,
        i32,
        Activity,
        activities,
        apk_id.eq
    );
    impl_get_multi_by!(pub get_providers_by_apk_id, i32, Provider, providers, apk_id.eq);

    impl_get_all!(pub get_diff_sources, DiffSource, diff_sources);
    impl_get_one_by!(pub
        get_diff_source_by_name,
        &str,
        DiffSource,
        diff_sources,
        name.eq
    );
    impl_delete_by!(pub
        delete_system_service_diff_by_service_id,
        i32,
        system_service_diffs,
        system_service.eq
    );

    impl_delete_by!(pub
        delete_system_service_methods_by_service_id,
        i32,
        system_service_methods,
        system_service_id.eq
    );

    impl_delete_by!(pub
        delete_system_service_impl_by_service_id,
        i32,
        system_service_impls,
        system_service_id.eq
    );
    impl_delete_by!(pub delete_diff_source_by_id, i32, diff_sources, id.eq);
    impl_delete_by!(pub delete_diff_source_by_name, &str, diff_sources, name.eq);

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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

    impl_diff_item!(pub
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
    impl_simple_gets!(pub
        fuzz_results,
        FuzzResult,
        get_fuzz_results,
        get_fuzz_result_by_id
    );
    impl_get_multi_by!(pub
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
    use crate::utils::ensure_dir_exists;

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

    fn db_test(context: &dyn Context, func: impl FnOnce(DeviceDatabase)) {
        let url = get_db_url(&context);
        let db = DeviceDatabase::new_from_url(&url).expect("failed to get database");
        let res = panic::catch_unwind(AssertUnwindSafe(|| func(db)));
        cleanup_database(&url);
        match res {
            Err(e) => panic::resume_unwind(e),
            _ => {}
        }
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
