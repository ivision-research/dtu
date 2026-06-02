use std::fs::File;
use std::str::FromStr;

use csv::StringRecord;
use diesel::connection::SimpleConnection;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::{insert_into, insert_or_ignore_into, prelude::*, sql_query, SqliteConnection};
use itertools::Itertools;
use smalisa::AccessFlag;

use super::common::*;
use super::models::{InsertClass, InsertLoadStatus, InsertSource};
use super::schema::{_load_status, classes, sources};
use super::setup_task::{AddDirTask, GraphDatabaseSetup, InitialImportOptions};
use super::FRAMEWORK_SOURCE;
use super::{setup::SetupResult, AddDirectoryOptions, SetupEvent};
use crate::db::graph::models::InsertDiscoveredString;
use crate::db::graph::schema::strings;
use crate::smalisa_wrapper::CSV;
use crate::utils::DevicePath;
use crate::{
    tasks::{EventMonitor, TaskCancelCheck},
    Context,
};

use super::common::Error;
use super::db::GraphSqliteDatabase;
use super::setup_task::*;

impl CSV {
    fn to_kind(self) -> i32 {
        (self as u8) as i32
    }
}

struct SetupContext<'a> {
    source: i32,
    data: &'a mut CsvReader,
    db: &'a GraphSqliteDatabase,
}

impl<'a> SetupContext<'a> {
    fn new(db: &'a GraphSqliteDatabase, source: i32, data: &'a mut CsvReader) -> Self {
        Self { source, data, db }
    }

    fn stage_methods(self) -> Result<()> {
        self.do_load(|c, record| -> Result<()> {
            let rp = RecordParser::new(record, CSV::Methods);
            let class = rp.get(0)?;
            let name = rp.get(1)?;
            let args = rp.get(2)?;
            let ret = rp.get(3)?;
            let access_flags: u64 = rp.get_parsable(4)?;

            let sql = "INSERT INTO named_methods(class, name, args, ret, access_flags) VALUES(?, ?, ?, ?, ?)";
            query!(sql_query(sql)
                .bind::<Text, _>(class)
                .bind::<Text, _>(name)
                .bind::<Text, _>(args)
                .bind::<Text, _>(ret)
                .bind::<BigInt, _>(access_flags as i64))
            .execute(c)?;
            Ok(())
        })
    }

    fn stage_method_field_access(self) -> Result<()> {
        self.do_load(|c, record| {
            let rp = RecordParser::new(record, CSV::MethodFieldAccess);
            let field_class = rp.get(0)?;
            let field_name = rp.get(1)?;
            let field_ty = rp.get(2)?;
            let method_class = rp.get(3)?;
            let method_name = rp.get(4)?;
            let method_args = rp.get(5)?;
            let op: i32 = rp.get_parsable(6)?;
            query!(sql_query(
                r#"INSERT INTO named_method_field_access(field_class, field_name, field_ty, method_class, method_name, method_args, action) VALUES(?, ?, ?, ?, ?, ?, ?)"#
            )
            .bind::<Text, _>(field_class)
            .bind::<Text, _>(field_name)
            .bind::<Text, _>(field_ty)
            .bind::<Text, _>(method_class)
            .bind::<Text, _>(method_name)
            .bind::<Text, _>(method_args)
            .bind::<Integer, _>(op))
            .execute(c)?;
            Ok(())
        })?;

        Ok(())
    }

    fn stage_class_fields(self) -> Result<()> {
        self.do_load(|c, record| {
            let rp = RecordParser::new(record, CSV::ClassFields);
            let  class = rp.get(0)?;
            let name = rp.get(1)?;
            let ty = rp.get(2)?;
            let access_flags: u64 = rp.get_parsable(3)?;
            query!(
                sql_query(r#"INSERT INTO named_class_fields(class, name, ty, access_flags) VALUES(?, ?, ?, ?)"#)
                    .bind::<Text, _>(class)
                    .bind::<Text, _>(name)
                    .bind::<Text, _>(ty)
                    .bind::<BigInt, _>(access_flags as i64)
            )
            .execute(c)?;
            Ok(())
        })?;
        Ok(())
    }

    fn stage_method_strings(self) -> Result<()> {
        self.do_load(|c, record| {
            let rp = RecordParser::new(record, CSV::MethodStrings);
            let string = rp.get(0)?;
            let method = rp.get(1)?;
            let method_args = rp.get(2)?;
            let class = rp.get(3)?;
            query!(
                sql_query(r#"INSERT INTO named_method_strings(string, method, method_args, class) VALUES(?, ?, ?, ?)"#)
                    .bind::<Text, _>(string)
                    .bind::<Text, _>(method)
                    .bind::<Text, _>(method_args)
                    .bind::<Text, _>(class)
            )
            .execute(c)?;
            Ok(())
        })?;
        Ok(())
    }

    fn load_classes(self) -> Result<()> {
        let src = self.source;
        self.do_load(|c, record| -> Result<()> {
            let ins = InsertClass::from_record(record, src)?;
            query!(insert_into(classes::table).values(&ins)).execute(c)?;
            Ok(())
        })
    }

    fn load_strings(self) -> Result<()> {
        let src = self.source;
        self.do_load(|c, record| -> Result<()> {
            let ins = InsertDiscoveredString::from_record(record, src)?;
            // We deduplicate over in gen_csvs but I dunno go ahead and do this so no funny
            // business?
            query!(insert_or_ignore_into(strings::table).values(&ins)).execute(c)?;
            Ok(())
        })
    }

    fn stage_calls(self) -> Result<()> {
        self.do_load(|c, record| -> Result<()> {
            let rp = RecordParser::new(record, CSV::Calls);
            let caller_class = rp.get(0)?;
            let caller_method = rp.get(1)?;
            let caller_args = rp.get(2)?;
            let callee_class = rp.get(3)?;
            let callee_method = rp.get(4)?;
            let callee_args = rp.get(5)?;

            let sql = r#"INSERT INTO named_calls(caller_class, caller_method, caller_args, callee_class, callee_method, callee_args) VALUES (?, ?, ?, ?, ?, ?)"#;
             query!(sql_query(sql)
                .bind::<Text, _>(caller_class)
                .bind::<Text, _>(caller_method)
                .bind::<Text, _>(caller_args)
                .bind::<Text, _>(callee_class)
                .bind::<Text, _>(callee_method)
                .bind::<Text, _>(callee_args))
            .execute(c)?;
            Ok(())
        })?;

        Ok(())
    }

    fn stage_supers(self) -> Result<()> {
        self.do_load(|c, record| -> Result<()> {
            let rp = RecordParser::new(record, CSV::Supers);
            let child = rp.get(0)?;
            let parent = rp.get(1)?;
            let sql = "INSERT INTO named_supers(parent, child) VALUES(?, ?)";
            query!(sql_query(sql)
                .bind::<Text, _>(parent)
                .bind::<Text, _>(child))
            .execute(c)?;
            Ok(())
        })
    }

    fn stage_impls(self) -> Result<()> {
        self.do_load(|c, record| -> Result<()> {
            let rp = RecordParser::new(record, CSV::Interfaces);
            let class = rp.get(0)?;
            let iface = rp.get(1)?;
            let sql = "INSERT INTO named_interfaces(interface, class) VALUES(?, ?)";
            query!(sql_query(sql).bind::<Text, _>(iface).bind::<Text, _>(class)).execute(c)?;
            Ok(())
        })
    }

    fn do_load<F>(self, f: F) -> Result<()>
    where
        F: Fn(&mut SqliteConnection, &StringRecord) -> Result<()> + Send,
    {
        Ok(self.db.transaction(move |c| -> Result<()> {
            let mut record = StringRecord::new();

            loop {
                match self.data.read_record(&mut record) {
                    Err(_) if self.data.is_done() => break,
                    Err(e) => return Err(Error::Generic(e.to_string())),
                    _ => {}
                }

                if record.is_empty() {
                    if self.data.is_done() {
                        break;
                    } else {
                        continue;
                    }
                }

                f(c, &record)?;

                record.clear();
            }

            Ok(())
        })?)
    }
}

impl GraphSqliteDatabase {
    fn finalize(&self, _ctx: &dyn Context) -> Result<()> {
        Ok(self.with_connection(|c| -> Result<()> {
            self.add_indices(c)?;
            Ok(())
        })?)
    }

    fn load_staged_method_field_access_with_conn(
        &self,
        conn: &mut SqliteConnection,
        src: i32,
    ) -> Result<()> {
        // A few separate parts to this query
        //
        // Use the staged raw string based values and do essentially two separate joins:
        //
        //  (1) Joining to get the field id from the `class_fields` table. This uses the string
        //      class name, field name, and field type to resolve the ID.
        //  (2) Joining to get the method id from the `methods` table. This uses the string method
        //      class name, method name, and method args to resolve the ID.
        query!(sql_query(
            r#"INSERT INTO method_field_access(field, method, action)
SELECT cf.id, m.id, acc.action
FROM named_method_field_access AS acc

JOIN classes AS field_classes
    ON field_classes.id = COALESCE(
        (SELECT id FROM classes WHERE name = acc.field_class AND source = ?1),
        (SELECT id FROM classes WHERE name = acc.field_class AND source = 1)
    )

JOIN class_fields AS cf
    ON cf.class = field_classes.id AND cf.name = acc.field_name AND cf.ty = acc.field_ty

JOIN classes AS method_classes
    ON method_classes.name = acc.method_class AND method_classes.source = ?1

JOIN methods AS m
    ON m.class = method_classes.id AND m.name = acc.method_name AND m.args = acc.method_args
"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        Ok(())
    }

    fn load_staged_class_fields_with_conn(
        &self,
        conn: &mut SqliteConnection,
        src: i32,
    ) -> Result<()> {
        // We can only discover class fields inside the source, so c.source should always give us
        // something unless some funny business has happened.
        query!(sql_query(
            r#"INSERT INTO class_fields(class, name, ty, access_flags)
SELECT c.id, ncf.name, ncf.ty, ncf.access_flags
FROM named_class_fields AS ncf
JOIN classes AS c
    ON c.name = ncf.class AND c.source = ?
"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        Ok(())
    }

    fn load_staged_supers_with_conn(&self, conn: &mut SqliteConnection, src: i32) -> Result<()> {
        // The `child` will already exist in the database, but the `parent` might not. If the parent
        // doesn't exist, add it to the framework, not the current source.
        query!(sql_query(
            r#"INSERT INTO classes(name, source)
    SELECT DISTINCT ns.parent, 1
    FROM named_supers AS ns
    LEFT JOIN classes AS c
        ON c.name = ns.parent AND (c.source = ? OR c.source = 1)
    WHERE c.name IS NULL"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;

        query!(sql_query(
            r#"INSERT INTO supers(parent, child, source)
SELECT DISTINCT parent.id, child.id, ?1
FROM named_supers AS ns
JOIN classes as child
    ON  child.name = ns.child
    AND child.source = ?1
JOIN classes AS parent
    ON parent.id = COALESCE(
        (SELECT id FROM classes WHERE name = ns.parent AND source = ?1),
        (SELECT id FROM classes WHERE name = ns.parent AND source = 1)
    )"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        Ok(())
    }

    fn load_staged_impls_with_conn(&self, conn: &mut SqliteConnection, src: i32) -> Result<()> {
        let flags = AccessFlag::PUBLIC | AccessFlag::INTERFACE;

        let raw_flags: i64 = flags.bits() as i64;

        // Similar to supers, the interface itself might not exist. Insert it into the framework in
        // that case and make sure interface is included in the access flags.
        query!(sql_query(
            r#"INSERT INTO classes(name, access_flags, source)
    SELECT DISTINCT ni.interface, ?, 1
    FROM named_interfaces AS ni
    LEFT JOIN classes AS c
        ON c.name = ni.interface AND (c.source = ? OR c.source = 1)
    WHERE c.name IS NULL"#
        )
        .bind::<BigInt, _>(raw_flags)
        .bind::<Integer, _>(src))
        .execute(conn)?;

        query!(sql_query(
            r#"INSERT INTO interfaces(interface, class, source)
SELECT DISTINCT interface.id, class.id, ?1
FROM named_interfaces AS ni
JOIN classes as class
    ON  class.name = ni.class
    AND class.source = ?1
JOIN classes AS interface
    ON interface.id = COALESCE(
        (SELECT id FROM classes WHERE name = ni.interface AND source = ?1),
        (SELECT id FROM classes WHERE name = ni.interface AND source = 1)
    )"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        Ok(())
    }

    fn load_staged_methods_with_conn(&self, conn: &mut SqliteConnection, src: i32) -> Result<()> {
        query!(sql_query(
            r#"INSERT INTO methods(class, name, args, ret, access_flags, source)
    SELECT DISTINCT c.id, nm.name, nm.args, nm.ret, nm.access_flags, ?1
    FROM named_methods AS nm
    JOIN classes AS c
        ON c.name = nm.class AND c.source = ?1"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        Ok(())
    }

    fn load_staged_method_strings_with_conn(
        &self,
        conn: &mut SqliteConnection,
        src: i32,
    ) -> Result<()> {
        // Since we allow duplicates of strings between sources, we can be sure the string is
        // available in this source: it can't possibly not be in the DB if a method in a given
        // source references it.
        query!(sql_query(
            r#"INSERT INTO method_strings(string, method)
    SELECT s.id, m.id
    FROM named_method_strings AS nms
    JOIN strings AS s
        ON s.string = nms.string AND s.source = ?1
    JOIN classes AS c
        ON c.name = nms.class AND c.source = ?1
    JOIN methods AS m
        ON m.class = c.id AND m.name = nms.method AND m.args = nms.method_args"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        Ok(())
    }

    fn load_staged_calls_with_conn(&self, conn: &mut SqliteConnection, src: i32) -> Result<()> {
        // The callee class might not exist. When that happens, we should add the class to the
        // database as part of the FRAMEWORK not as part of our current source. If it was part of
        // the current source we should have already added it when we added classes for this source,
        // which always should happen first.
        let new_classes = query!(sql_query(
            r#"INSERT INTO classes(name, source)
    SELECT DISTINCT nc.callee_class, 1
        FROM named_calls AS nc
        LEFT JOIN classes AS c
            ON c.name = nc.callee_class AND (c.source = ? OR c.source = 1)
        WHERE c.name IS NULL"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        if new_classes > 0 {
            log::debug!("Found {new_classes} new classes via the calls.csv callees");
        }

        // The callee class will now always exist, but the method itself might not.
        //
        // TODO: I'm not sure if we need to run this query if `new_classes == 0`. I think we
        // might, so I'm leaving it as is, but it could be worth investigating at some point.
        let new_methods = query!(sql_query(
            r#"INSERT INTO methods(class, name, args, ret, source)
    SELECT DISTINCT c.id, nc.callee_method, nc.callee_args, 'V', ?1
    FROM named_calls AS nc
    JOIN classes AS c
        ON c.id = COALESCE(
            (SELECT id FROM classes WHERE name = nc.callee_class AND source = ?1),
            (SELECT id FROM classes WHERE name = nc.callee_class AND source = 1)
        )
    LEFT JOIN methods AS m
        ON m.class = c.id AND m.name = nc.callee_method AND m.args = nc.callee_args
    WHERE m.name IS NULL"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;
        if new_classes > 0 {
            log::debug!("Found {new_methods} new methods via the calls.csv callees");
        }

        // Next populate the calls table
        //
        // Some notes on this insert:
        //
        // 1. The caller's class will always share the same source as the method, so
        //    the join on methods doesn't need the source involved, as this comes
        //    with the class ID
        // 2. The callee's class and method will either always be in the call source or the
        //    framework, this is ensured by the INSERTs above.
        // 3. We prevent simple recursive calls here. We're still very likely to have cycles
        //    on the call graph even with this step, but at least it's something

        query!(sql_query(
            r#"
INSERT INTO calls(caller, callee, source)
SELECT DISTINCT src.id, dst.id, ?1
FROM named_calls AS nc

JOIN classes AS sc
    ON sc.name = nc.caller_class AND sc.source = ?1

JOIN methods AS src 
    ON  src.class = sc.id
    AND src.name = nc.caller_method
    AND src.args = nc.caller_args

JOIN classes AS dc
    ON dc.id = COALESCE(
        (SELECT id FROM classes WHERE name = nc.callee_class AND source = ?1),
        (SELECT id FROM classes WHERE name = nc.callee_class AND source = 1)
    )

JOIN methods AS dst
    ON  dst.class = dc.id
    AND dst.name = nc.callee_method
    AND dst.args = nc.callee_args

WHERE dst.id != src.id"#
        )
        .bind::<Integer, _>(src))
        .execute(conn)?;

        Ok(())
    }

    fn load_staged_impls(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_impls_with_conn(c, src))?)
    }

    fn load_staged_method_strings(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_method_strings_with_conn(c, src))?)
    }

    fn load_staged_method_field_access(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_method_field_access_with_conn(c, src))?)
    }

    fn load_staged_class_fields(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_class_fields_with_conn(c, src))?)
    }

    fn load_staged_supers(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_supers_with_conn(c, src))?)
    }

    fn load_staged_methods(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_methods_with_conn(c, src))?)
    }

    fn load_staged_calls(&self, src: i32) -> Result<()> {
        Ok(self.transaction(|c| self.load_staged_calls_with_conn(c, src))?)
    }

    fn add_indices(&self, conn: &mut SqliteConnection) -> Result<()> {
        log::debug!("Creating post setup indices");
        Ok(conn.batch_execute(
            r#"
                CREATE INDEX IF NOT EXISTS source ON sources(name);

                CREATE INDEX IF NOT EXISTS class_source ON classes(source);
                CREATE INDEX IF NOT EXISTS methods_class ON methods(class);
                CREATE INDEX IF NOT EXISTS methods_source ON methods(source);
                CREATE INDEX IF NOT EXISTS methods_name ON methods(name);

                CREATE INDEX IF NOT EXISTS calls_callee_source ON calls(callee, source);
                CREATE INDEX IF NOT EXISTS calls_caller_source ON calls(caller, source);

                CREATE INDEX IF NOT EXISTS supers_parent_source ON supers(parent, source);
                CREATE INDEX IF NOT EXISTS supers_child_source ON supers(child, source);

                CREATE INDEX IF NOT EXISTS interfaces_parent_source ON interfaces(interface, source);
                CREATE INDEX IF NOT EXISTS interfaces_child_source ON interfaces(class, source);

                CREATE INDEX IF NOT EXISTS method_strings_method ON method_strings(method);
                CREATE INDEX IF NOT EXISTS method_strings_strings ON method_strings(string);

                CREATE INDEX IF NOT EXISTS method_field_access_method ON method_field_access(method);

                CREATE INDEX IF NOT EXISTS class_fields_class ON class_fields(class);
                "#,
        )?)
    }

    fn update_load_status(conn: &mut SqliteConnection, src: i32, status: CSV) -> Result<()> {
        let ls = InsertLoadStatus::new(src, status.to_kind());
        _ = query!(insert_into(_load_status::table).values(&ls)).execute(conn)?;
        Ok(())
    }
}

type CsvReader = csv::Reader<File>;

impl GraphDatabaseSetup for GraphSqliteDatabase {
    fn run_initial_import(
        &self,
        ctx: &dyn Context,
        opts: InitialImportOptions,
        monitor: &dyn EventMonitor<SetupEvent>,
        cancel: &TaskCancelCheck,
    ) -> SetupResult<()> {
        log::debug!("Starting initial import with framework");
        let framework_dir = ctx.get_smalisa_analysis_dir()?.join("framework");
        let add_opts = AddDirectoryOptions::new(FRAMEWORK_SOURCE.into(), &framework_dir);
        self.add_directory(ctx, add_opts, monitor, cancel)?;

        log::debug!("Starting import of APKs");
        let apks = opts.get_apk_smalisa_dirs(ctx)?;

        for apk in apks {
            let device_path = DevicePath::from_path(&apk)?;
            log::trace!("Starting APK {}", device_path);
            let add_opts = AddDirectoryOptions::new(device_path.get_squashed_string(), &apk);
            self.add_directory(ctx, add_opts, monitor, cancel)?;
        }

        monitor.on_event(SetupEvent::Finalizing);

        self.finalize(ctx)?;

        Ok(())
    }

    fn add_directory(
        &self,
        ctx: &dyn Context,
        opts: AddDirectoryOptions,
        monitor: &dyn EventMonitor<SetupEvent>,
        cancel: &TaskCancelCheck,
    ) -> SetupResult<()> {
        self.with_connection(|c| -> std::result::Result<(), Error> {
            let ins = InsertSource::new(&opts.name);
            _ = insert_into(sources::table)
                .values(&ins)
                .on_conflict_do_nothing()
                .execute(c);
            Ok(())
        })?;

        let task = AddDirTask {
            ctx,
            cancel,
            opts,
            monitor,
            graph: self,
        };

        task.run()?;
        Ok(())
    }

    fn load_csv(&self, _ctx: &dyn Context, path: &str, source: &str, kind: CSV) -> Result<()> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)
            .map_err(|e| {
                Error::Generic(format!(
                    "failed to open {path} (source {source}) as a csv: {e}"
                ))
            })?;

        let src = self.get_source_id(source)?;

        let setup = SetupContext::new(self, src, &mut reader);

        match kind {
            CSV::Interfaces => {
                setup.stage_impls()?;
                self.load_staged_impls(src)?;
            }
            CSV::Supers => {
                setup.stage_supers()?;
                self.load_staged_supers(src)?;
            }
            CSV::Calls => {
                setup.stage_calls()?;
                self.load_staged_calls(src)?;
            }
            CSV::Methods => {
                setup.stage_methods()?;
                self.load_staged_methods(src)?;
            }
            CSV::Classes => {
                setup.load_classes()?;
            }
            CSV::Strings => {
                setup.load_strings()?;
            }
            CSV::ClassFields => {
                setup.stage_class_fields()?;
                self.load_staged_class_fields(src)?;
            }
            CSV::MethodFieldAccess => {
                setup.stage_method_field_access()?;
                self.load_staged_method_field_access(src)?;
            }
            CSV::MethodStrings => {
                setup.stage_method_strings()?;
                self.load_staged_method_strings(src)?;
            }
        }

        Ok(self.with_connection(|c| Self::update_load_status(c, src, kind))?)
    }

    fn load_begin(&self, _ctx: &dyn Context) -> Result<()> {
        log::trace!("Setting up the temporary tables for loading");
        // When starting the load, we want to change some settings for performance. In particular,
        // we disable foreign keys. Then we create temporary tables for storing nodes by name
        self.with_connection(|c| {
            c.batch_execute(
                r#"PRAGMA synchronous=NORMAL;
PRAGMA temp_store=MEMORY;
PRAGMA foreign_keys=OFF;

CREATE TEMPORARY TABLE IF NOT EXISTS named_method_field_access(
    field_class TEXT NOT NULL,
    field_name TEXT NOT NULL,
    field_ty TEXT NOT NULL,
    method_class TEXT NOT NULL,
    method_name TEXT NOT NULL,
    method_args TEXT NOT NULL,
    action INTEGER NOT NULL
);

CREATE TEMPORARY TABLE IF NOT EXISTS named_class_fields(
    class TEXT NOT NULL,
    name TEXT NOT NULL,
    ty TEXT NOT NULL,
    access_flags BIGINT NOT NULL
);

CREATE TEMPORARY TABLE IF NOT EXISTS named_method_strings(
    string TEXT NOT NULL,
    method TEXT NOT NULL,
    method_args TEXT NOT NULL,
    class TEXT NOT NULL
);

CREATE TEMPORARY TABLE IF NOT EXISTS named_methods(
    class TEXT NOT NULL,
    name TEXT NOT NULL,
    args TEXT NOT NULL,
    ret TEXT NOT NULL,
    access_flags BIGINT NOT NULL
);

CREATE TEMPORARY TABLE IF NOT EXISTS named_calls(
    caller_class TEXT NOT NULL,
    caller_method TEXT NOT NULL,
    caller_args TEXT NOT NULL,

    callee_class TEXT NOT NULL,
    callee_method TEXT NOT NULL,
    callee_args TEXT NOT NULL
);

CREATE TEMPORARY TABLE IF NOT EXISTS named_supers(
    parent TEXT NOT NULL,
    child TEXT NOT NULL
);

CREATE TEMPORARY TABLE IF NOT EXISTS named_interfaces(
    interface TEXT NOT NULL,
    class TEXT NOT NULL
);
"#,
            )?;
            Ok(())
        })
    }

    fn load_complete(&self, _ctx: &dyn Context, _success: bool) -> Result<()> {
        self.with_connection(|c| {
            c.batch_execute(
                r#"
            DELETE FROM named_method_field_access;
            DELETE FROM named_class_fields;
            DELETE FROM named_method_strings;
            DELETE FROM named_calls;
            DELETE FROM named_methods;
            DELETE FROM named_supers;
            DELETE FROM named_interfaces;
                "#,
            )?;
            Ok(())
        })
    }

    fn should_load_csv(&self, source: &str, csv: CSV) -> bool {
        let kind = csv.to_kind();
        self.with_connection(|c| {
            query!(_load_status::table
                .inner_join(sources::table)
                .filter(sources::name.eq(source))
                .filter(_load_status::kind.eq(kind))
                .select(_load_status::rowid)
                .limit(1))
            .get_result::<i32>(c)
            .is_err()
        })
    }
}

struct RecordParser<'a> {
    record: &'a StringRecord,
    kind: CSV,
}

impl<'a> RecordParser<'a> {
    fn new(record: &'a StringRecord, kind: CSV) -> Self {
        Self { record, kind }
    }
    fn get(&self, idx: usize) -> Result<&'a str> {
        self.record.get(idx).ok_or_else(|| {
            Error::Generic(format!(
                "invalid {}, missing string at {} - line = {}",
                self.kind.file_name(),
                idx,
                self.record.iter().join(" | ")
            ))
        })
    }
    fn get_parsable<T>(&self, idx: usize) -> Result<T>
    where
        T: FromStr,
    {
        let val = self.get(idx)?;
        str::parse::<T>(val).map_err(|_| {
            Error::Generic(format!(
                "invalid {}, failed to parse value `{}` at {}",
                self.kind.file_name(),
                val,
                idx
            ))
        })
    }
}

impl<'a> InsertDiscoveredString<'a> {
    fn from_record(record: &'a StringRecord, src: i32) -> Result<Self> {
        let rp = RecordParser::new(record, CSV::Strings);
        let s = rp.get(0)?;
        Ok(Self::new(s, src))
    }
}

impl<'a> InsertClass<'a> {
    fn from_record(record: &'a StringRecord, src: i32) -> Result<Self> {
        let rp = RecordParser::new(record, CSV::Classes);
        let name = rp.get(0)?;
        let raw_flags = rp.get_parsable::<u64>(1)?;
        let flags = AccessFlag::from_bits_truncate(raw_flags);
        Ok(Self::new(name, flags.bits() as i64, src))
    }
}
