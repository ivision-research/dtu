use std::ops::Deref;

use diesel::prelude::*;
use diesel::{delete, insert_into, update};
use diesel_migrations::{embed_migrations, EmbeddedMigrations};

use super::schema::*;
use crate::Context;

use super::common::*;
use super::models::*;
use super::schema;
use crate::db::macros::{
    def_delete_by, def_get_multi, def_get_one_by, def_insert_multi, def_insert_one, def_update_one,
    impl_delete_by, impl_get_all, impl_get_multi, impl_get_one_by, impl_insert_multi,
    impl_insert_one, impl_update_one, query,
};
use crate::prereqs::Prereq;

pub const APP_ID_KEY: &'static str = "app_id";

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/meta_migrations/");

#[cfg(test)]
const TEST_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/test_meta_migrations/");

pub static META_DATABASE_FILE_NAME: &'static str = "meta.db";

pub trait Database: Sync + Sync {
    fn prereq_done(&self, prereq: Prereq) -> Result<bool>;
    fn ensure_prereq(&self, prereq: Prereq) -> crate::Result<()>;
    fn update_prereq(&self, prereq: Prereq, completed: bool) -> Result<()>;

    def_get_one_by!(get_progress, Prereq, ProgressStep);
    def_update_one!(update_progress, ProgressStep);
    def_get_multi!(get_all_progress, ProgressStep);

    def_insert_one!(add_decompile_status, InsertDecompileStatus);
    def_get_one_by!(get_decompile_status_by_device_path, &str, DecompileStatus);
    def_update_one!(update_decompile_status, DecompileStatus);
    def_delete_by!(delete_decompile_status_by_id, i32);

    fn wipe_app_data(&self) -> Result<()>;

    fn set_app_permission_usability(&self, name: &str, usable: bool) -> Result<()>;
    def_insert_one!(add_app_permission, InsertAppPermission);
    def_insert_multi!(add_app_permissions, InsertAppPermission);
    def_get_multi!(get_usable_app_permissions, AppPermission);
    def_update_one!(update_app_permission, AppPermission);
    def_delete_by!(delete_app_permission_by_id, i32);

    def_insert_one!(add_app_activity, InsertAppActivity);
    def_insert_multi!(add_app_activities, InsertAppActivity);
    def_get_multi!(get_app_activities, AppActivity);
    def_update_one!(update_app_activity, AppActivity);
    def_delete_by!(delete_app_activity_by_name, &str);
    def_delete_by!(delete_app_activity_by_id, i32);
    def_get_one_by!(get_app_activity_by_name, &str, AppActivity);

    fn app_activity_name_taken(&self, name: &str) -> Result<bool>;

    fn get_key_value(&self, key: &str) -> Result<String>;
    fn add_key_value(&self, key: &str, value: &str) -> Result<()>;
    fn update_key_value(&self, key: &str, value: &str) -> Result<()>;
    fn delete_key_value(&self, key: &str) -> Result<()>;
}

pub struct MetaSqliteDatabase {
    db: Db,
}

impl Deref for MetaSqliteDatabase {
    type Target = Db;
    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl MetaSqliteDatabase {
    pub fn new(ctx: &dyn Context) -> Result<Self> {
        Ok(Self {
            db: Db::new(
                ctx,
                META_DATABASE_FILE_NAME,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        })
    }

    pub fn new_from_path<S: AsRef<str> + ?Sized>(path: &S) -> Result<Self> {
        Ok(Self {
            db: Db::new_from_path(
                path,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        })
    }

    #[allow(dead_code)]
    fn new_from_url(url: &String) -> Result<Self> {
        Ok(Self {
            db: Db::new_from_url(
                url,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        })
    }
}

impl Database for MetaSqliteDatabase {
    impl_insert_one!(
        add_decompile_status,
        InsertDecompileStatus,
        decompile_status
    );
    impl_get_one_by!(
        get_decompile_status_by_device_path,
        &str,
        DecompileStatus,
        decompile_status,
        device_path.eq
    );
    impl_update_one!(update_decompile_status, DecompileStatus, decompile_status);
    impl_delete_by!(delete_decompile_status_by_id, i32, decompile_status, id.eq);

    fn get_progress(&self, sel: Prereq) -> Result<ProgressStep> {
        Ok(self.query(|conn| {
            query!(progress::table.filter(progress::step.eq(sel)).limit(1))
                .get_result::<(i32, Prereq, bool)>(conn)
                .map(|(_, step, completed)| ProgressStep { step, completed })
        })?)
    }

    fn get_all_progress(&self) -> Result<Vec<ProgressStep>> {
        Ok(self.query(|conn| {
            progress::table.load::<(i32, Prereq, bool)>(conn).map(|it| {
                it.into_iter()
                    .map(|(_, step, completed)| ProgressStep { step, completed })
                    .collect()
            })
        })?)
    }

    fn update_progress(&self, prog: &ProgressStep) -> Result<()> {
        self.write(|conn| {
            query!(update(progress::table.filter(progress::step.eq(prog.step)))
                .set(progress::completed.eq(prog.completed)))
            .execute(conn)?;
            Ok(())
        })
    }

    fn prereq_done(&self, prereq: Prereq) -> Result<bool> {
        Ok(self.get_progress(prereq)?.completed)
    }

    fn ensure_prereq(&self, prereq: Prereq) -> crate::Result<()> {
        let done = self
            .prereq_done(prereq)
            .map_err(|e| crate::Error::Generic(e.to_string()))?;
        if done {
            Ok(())
        } else {
            Err(crate::Error::UnsatisfiedPrereq(prereq))
        }
    }

    fn update_prereq(&self, prereq: Prereq, completed: bool) -> Result<()> {
        let prog = ProgressStep {
            step: prereq,
            completed,
        };
        self.update_progress(&prog)
    }

    fn set_app_permission_usability(&self, name: &str, is_usable: bool) -> Result<()> {
        self.write(|conn| {
            query!(
                update(app_permissions::table.filter(app_permissions::permission.eq(name)))
                    .set(app_permissions::usable.eq(is_usable))
            )
            .execute(conn)?;
            Ok(())
        })
    }
    impl_insert_one!(add_app_permission, InsertAppPermission, app_permissions);
    impl_insert_multi!(add_app_permissions, InsertAppPermission, app_permissions);
    impl_get_multi!(
        get_usable_app_permissions,
        AppPermission,
        app_permissions,
        usable.eq(true)
    );
    impl_update_one!(update_app_permission, AppPermission, app_permissions);
    impl_delete_by!(delete_app_permission_by_id, i32, app_permissions, id.eq);

    impl_insert_one!(add_app_activity, InsertAppActivity, app_activities);
    impl_insert_multi!(add_app_activities, InsertAppActivity, app_activities);
    impl_get_all!(get_app_activities, AppActivity, app_activities);
    impl_update_one!(update_app_activity, AppActivity, app_activities);
    impl_delete_by!(delete_app_activity_by_name, &str, app_activities, name.eq);
    impl_delete_by!(delete_app_activity_by_id, i32, app_activities, id.eq);
    impl_get_one_by!(
        get_app_activity_by_name,
        &str,
        AppActivity,
        app_activities,
        name.eq
    );

    fn app_activity_name_taken(&self, check_name: &str) -> Result<bool> {
        Ok(self
            .query(|c| {
                query!(app_activities::table.filter(app_activities::name.eq(check_name)))
                    .get_result::<AppActivity>(c)
                    .optional()
            })?
            .is_some())
    }

    fn wipe_app_data(&self) -> Result<()> {
        self.write(|conn| {
            delete(schema::app_activities::dsl::app_activities).execute(conn)?;
            delete(schema::app_permissions::dsl::app_permissions).execute(conn)?;
            Ok(())
        })
    }

    fn add_key_value(&self, key: &str, value: &str) -> Result<()> {
        let ins = InsertKeyValue { key, value };
        self.write(|c| {
            query!(insert_into(key_values::table).values(&ins)).execute(c)?;
            Ok(())
        })
    }

    fn update_key_value(&self, key: &str, value: &str) -> Result<()> {
        use super::schema::key_values::dsl;
        self.write(|c| {
            query!(update(dsl::key_values)
                .filter(dsl::key.eq(key))
                .set(dsl::value.eq(value)))
            .execute(c)?;
            Ok(())
        })
    }

    fn get_key_value(&self, key: &str) -> Result<String> {
        Ok(self
            .query(|c| {
                query!(key_values::table.filter(key_values::key.eq(key))).get_result::<KeyValue>(c)
            })
            .map(|it| it.value)?)
    }

    fn delete_key_value(&self, key: &str) -> Result<()> {
        self.write(|c| {
            query!(delete(key_values::table.filter(key_values::key.eq(key)))).execute(c)?;
            Ok(())
        })
    }
}
