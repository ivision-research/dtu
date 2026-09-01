use std::fs::File;
use std::str::FromStr;

use csv::StringRecord;
use diesel::connection::SimpleConnection;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::{insert_into, insert_or_ignore_into, prelude::*, sql_query, update, SqliteConnection};
use itertools::Itertools;
use smalisa::AccessFlag;

use super::common::*;
use super::models::{InsertClass, InsertLoadStatus, InsertSource};
use super::schema::{_load_status, classes, sources};
use super::setup_task::{AddDirTask, GraphDatabaseSetup, InitialImportOptions};
use super::FRAMEWORK_SOURCE;
use super::{setup::SetupResult, AddDirectoryOptions, SetupEvent};
use crate::db::graph::models::{InsertDiscoveredString, SourceId};
use crate::db::graph::schema::{_metadata, strings};
use crate::db::macros::query;
use crate::smalisa_wrapper::CSV;
use crate::utils::{unix_now, DevicePath};
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

/// Applied to the load connection before its transaction opens, and undone after
///
/// Many of these are just for performance since we're doing large loads. For example turning
/// foreign keys off shaves about 20% off the runtime for load, and we know the keys will be
/// consistent so it's fine to turn it off.
const CSV_LOAD_PRAGMAS: &str = "\
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = OFF;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;";

/// Staging tables holding a CSV's rows by name until they are resolved to ids
///
/// These are temporary, so they belong to one connection and are only visible for as long
/// as the load holds it.
const CREATE_STAGING_TABLES: &str = r#"
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
"#;

const DROP_STAGING_TABLES: &str = r#"
DROP TABLE IF EXISTS named_method_field_access;
DROP TABLE IF EXISTS named_class_fields;
DROP TABLE IF EXISTS named_method_strings;
DROP TABLE IF EXISTS named_methods;
DROP TABLE IF EXISTS named_calls;
DROP TABLE IF EXISTS named_supers;
DROP TABLE IF EXISTS named_interfaces;
"#;

struct SetupContext<'a> {
    source: SourceId,
    data: &'a mut CsvReader,
}

impl<'a> SetupContext<'a> {
    fn new(source: SourceId, data: &'a mut CsvReader) -> Self {
        Self { source, data }
    }

    fn stage_methods(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| -> Result<()> {
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

    fn stage_method_field_access(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| {
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

    fn stage_class_fields(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| {
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

    fn stage_method_strings(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| {
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

    fn load_classes(self, conn: &mut SqliteConnection) -> Result<()> {
        let src = self.source;
        self.do_load(conn, |c, record| -> Result<()> {
            let ins = InsertClass::from_record(record, src)?;
            query!(insert_into(classes::table).values(&ins)).execute(c)?;
            Ok(())
        })
    }

    fn load_strings(self, conn: &mut SqliteConnection) -> Result<()> {
        let src = self.source;
        self.do_load(conn, |c, record| -> Result<()> {
            let ins = InsertDiscoveredString::from_record(record, src)?;
            // We deduplicate over in gen_csvs but I dunno go ahead and do this so no funny
            // business?
            query!(insert_or_ignore_into(strings::table).values(&ins)).execute(c)?;
            Ok(())
        })
    }

    fn stage_calls(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| -> Result<()> {
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

    fn stage_supers(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| -> Result<()> {
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

    fn stage_impls(self, conn: &mut SqliteConnection) -> Result<()> {
        self.do_load(conn, |c, record| -> Result<()> {
            let rp = RecordParser::new(record, CSV::Interfaces);
            let class = rp.get(0)?;
            let iface = rp.get(1)?;
            let sql = "INSERT INTO named_interfaces(interface, class) VALUES(?, ?)";
            query!(sql_query(sql).bind::<Text, _>(iface).bind::<Text, _>(class)).execute(c)?;
            Ok(())
        })
    }

    fn do_load<F>(self, conn: &mut SqliteConnection, f: F) -> Result<()>
    where
        F: Fn(&mut SqliteConnection, &StringRecord) -> Result<()>,
    {
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

            f(conn, &record)?;

            record.clear();
        }

        Ok(())
    }
}

impl GraphSqliteDatabase {
    fn finalize(&self, _ctx: &dyn Context) -> Result<()> {
        self.write(|c| {
            Self::add_indices(c)?;
            Self::update_built_at(c)
        })
    }

    /// Record that the graph finished building
    ///
    /// Ids are only stable for one build, so anything holding them outside this database
    /// compares this timestamp against the one it saw. Adding a single directory
    /// deliberately leaves it alone: appending a source hands out new ids without
    /// renumbering the existing ones, so an artifact built before it is still readable.
    fn update_built_at(conn: &mut SqliteConnection) -> Result<()> {
        query!(update(_metadata::table).set(_metadata::built_at.eq(unix_now()?))).execute(conn)?;
        Ok(())
    }

    fn load_staged_method_field_access(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn load_staged_class_fields(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn load_staged_supers(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn load_staged_impls(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn load_staged_methods(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn load_staged_method_strings(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn load_staged_calls(conn: &mut SqliteConnection, src: SourceId) -> Result<()> {
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

    fn add_indices(conn: &mut SqliteConnection) -> Result<()> {
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

                ANALYZE;
                "#,
        )?)
    }

    fn update_load_status(conn: &mut SqliteConnection, src: SourceId, status: CSV) -> Result<()> {
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
        self.write(|c| -> Result<()> {
            let ins = InsertSource::new(&opts.name);
            insert_into(sources::table)
                .values(&ins)
                .on_conflict_do_nothing()
                .execute(c)?;
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

        // One CSV is one write: the staging tables are created, filled, drained into the
        // real tables and dropped on a single connection inside a single transaction. The
        // staging tables are per connection, so nothing here may reach for another one.
        self.write_with_pragmas(CSV_LOAD_PRAGMAS, |conn| {
            conn.batch_execute(CREATE_STAGING_TABLES)?;

            let setup = SetupContext::new(src, &mut reader);

            match kind {
                CSV::Interfaces => {
                    setup.stage_impls(conn)?;
                    Self::load_staged_impls(conn, src)?;
                }
                CSV::Supers => {
                    setup.stage_supers(conn)?;
                    Self::load_staged_supers(conn, src)?;
                }
                CSV::Calls => {
                    setup.stage_calls(conn)?;
                    Self::load_staged_calls(conn, src)?;
                }
                CSV::Methods => {
                    setup.stage_methods(conn)?;
                    Self::load_staged_methods(conn, src)?;
                }
                CSV::Classes => setup.load_classes(conn)?,
                CSV::Strings => setup.load_strings(conn)?,
                CSV::ClassFields => {
                    setup.stage_class_fields(conn)?;
                    Self::load_staged_class_fields(conn, src)?;
                }
                CSV::MethodFieldAccess => {
                    setup.stage_method_field_access(conn)?;
                    Self::load_staged_method_field_access(conn, src)?;
                }
                CSV::MethodStrings => {
                    setup.stage_method_strings(conn)?;
                    Self::load_staged_method_strings(conn, src)?;
                }
            }

            Self::update_load_status(conn, src, kind)?;

            // The connection goes back to the pool, so the staged rows have to go with it
            conn.batch_execute(DROP_STAGING_TABLES)?;
            Ok(())
        })
    }

    fn should_load_csv(&self, source: &str, csv: CSV) -> Result<bool> {
        let kind = csv.to_kind();
        Ok(self.query(|c| {
            (_load_status::table
                .inner_join(sources::table)
                .filter(sources::name.eq(source))
                .filter(_load_status::kind.eq(kind))
                .select(_load_status::rowid)
                .limit(1))
            .get_result::<i32>(c)
            .optional()
            .map(|it| it.is_none())
        })?)
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
    fn from_record(record: &'a StringRecord, src: SourceId) -> Result<Self> {
        let rp = RecordParser::new(record, CSV::Strings);
        let s = rp.get(0)?;
        Ok(Self::new(s, src))
    }
}

impl<'a> InsertClass<'a> {
    fn from_record(record: &'a StringRecord, src: SourceId) -> Result<Self> {
        let rp = RecordParser::new(record, CSV::Classes);
        let name = rp.get(0)?;
        let raw_flags = rp.get_parsable::<u64>(1)?;
        let flags = AccessFlag::from_bits_truncate(raw_flags);
        Ok(Self::new(name, flags.bits() as i64, src))
    }
}

#[cfg(test)]
mod test {
    use std::path::{Path, PathBuf};

    use diesel::dsl::sql;
    use diesel::sql_types::{BigInt, Nullable};
    use rstest::*;

    use super::super::models::{
        ClassId, FieldAccessOp, MethodId, MethodSearch, MethodSearchParams,
    };
    use super::super::schema::methods;
    use super::super::{ClassSearch, GraphDatabase};
    use super::*;
    use crate::testing::{tmp_context, TestContext};
    use crate::utils::ensure_dir_exists;
    use crate::utils::ClassName;

    fn sorted<I: Iterator<Item = String>>(it: I) -> Vec<String> {
        let mut out = it.collect::<Vec<String>>();
        out.sort();
        out
    }

    fn write_csv(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).expect("failed to write the test csv");
        path
    }

    fn load(db: &GraphSqliteDatabase, ctx: &dyn Context, path: &Path, source: &str, kind: CSV) {
        db.load_csv(ctx, path.to_str().unwrap(), source, kind)
            .unwrap_or_else(|e| {
                panic!("failed to load {} for {}: {}", kind.file_name(), source, e)
            });
    }

    /// The methods the source declares on `class`, which the test fixture never touches
    fn method_names(db: &GraphSqliteDatabase, source: &str, class: &str) -> Vec<String> {
        db.query(|c| {
            let sid = sources::table
                .filter(sources::name.eq(source))
                .select(sources::id)
                .first::<SourceId>(c)?;
            let cid = classes::table
                .filter(classes::name.eq(class))
                .filter(classes::source.eq(sid))
                .select(classes::id)
                .first::<ClassId>(c)?;
            Ok(methods::table
                .filter(methods::class.eq(cid))
                .select(methods::name)
                .load::<String>(c)?)
        })
        .expect("failed to read back the methods")
    }

    /// Every CSV for one small source, in the order the importer loads them
    ///
    /// `Lz/A;` implements `Lz/Iface;`, `Lz/B;` extends it, `alpha` calls `Lz/B;->gamma`
    /// and a method on a class this source never declares, reads the string
    /// `hello-ingest` and writes the field `Lz/A;->field1:I`.
    const SOURCE: &str = "ingest-apk";

    fn load_all(db: &GraphSqliteDatabase, ctx: &dyn Context, dir: &Path) {
        for (kind, contents) in [
            (CSV::Classes, "Lz/A;,1\nLz/B;,1\nLz/Iface;,1536\n"),
            (CSV::Strings, "hello-ingest\n"),
            (
                CSV::Methods,
                "Lz/A;,alpha,,V,1\nLz/A;,beta,Ljava/lang/String;,Z,1\nLz/B;,gamma,,V,1\n",
            ),
            (CSV::Supers, "Lz/B;,Lz/A;\n"),
            (CSV::Interfaces, "Lz/A;,Lz/Iface;\n"),
            (
                CSV::Calls,
                "Lz/A;,alpha,,Lz/B;,gamma,\nLz/A;,alpha,,Lz/Absent;,missing,\n",
            ),
            (CSV::ClassFields, "Lz/A;,field1,I,2\n"),
            (CSV::MethodStrings, "hello-ingest,alpha,,Lz/A;\n"),
            (CSV::MethodFieldAccess, "Lz/A;,field1,I,Lz/A;,alpha,,1\n"),
        ] {
            let path = write_csv(dir, kind.file_name(), contents);
            load(db, ctx, &path, SOURCE, kind);
        }
    }

    fn import_dir(ctx: &dyn Context) -> PathBuf {
        let dir = ctx
            .get_graph_import_dir()
            .expect("failed to get the import dir");
        ensure_dir_exists(&dir).expect("failed to make the import dir");
        dir
    }

    fn add_source(db: &GraphSqliteDatabase, name: &str) {
        db.write(|c| -> Result<()> {
            insert_into(sources::table)
                .values(&InsertSource::new(name))
                .execute(c)?;
            Ok(())
        })
        .expect("failed to add the source");
    }

    fn method_id(db: &GraphSqliteDatabase, source: &str, class: &str, name: &str) -> MethodId {
        let class = ClassName::from(class);
        let params = MethodSearchParams::new(Some(name), Some(&class), None)
            .expect("bad method search params");
        let search = MethodSearch::from(params).with_source(source);
        let found = db.get_methods(&search).expect("get_methods failed");
        assert_eq!(found.len(), 1, "expected one {} in {}", name, source);
        found[0].id
    }

    /// Loading two sources must not let one source's staged rows reach the other
    ///
    /// The staging tables are temporary, so they belong to whichever pooled connection the
    /// load ran on. If a load left rows behind, the next load to be handed that connection
    /// would insert them again under its own source.
    #[rstest]
    fn test_load_csv_does_not_leak_staged_rows(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");

        let dir = ctx
            .get_graph_import_dir()
            .expect("failed to get the import dir");
        ensure_dir_exists(&dir).expect("failed to make the import dir");

        db.write(|c| -> Result<()> {
            insert_into(sources::table)
                .values(&InsertSource::new("apk"))
                .execute(c)?;
            Ok(())
        })
        .expect("failed to add the second source");

        // Both sources declare the same class, and each declares a method the other does
        // not, so a staged row that outlives its load shows up as an extra method.
        let classes = write_csv(&dir, "classes.csv", "Lcom/a/A;,1\n");
        let framework_methods = write_csv(&dir, "fw-methods.csv", "Lcom/a/A;,fromFramework,,V,1\n");
        let apk_methods = write_csv(&dir, "apk-methods.csv", "Lcom/a/A;,fromApk,,V,1\n");

        load(&db, ctx, &classes, FRAMEWORK_SOURCE, CSV::Classes);
        load(&db, ctx, &framework_methods, FRAMEWORK_SOURCE, CSV::Methods);
        load(&db, ctx, &classes, "apk", CSV::Classes);
        load(&db, ctx, &apk_methods, "apk", CSV::Methods);

        assert_eq!(
            method_names(&db, FRAMEWORK_SOURCE, "Lcom/a/A;"),
            vec!["fromFramework"]
        );
        assert_eq!(method_names(&db, "apk", "Lcom/a/A;"), vec!["fromApk"]);
    }

    /// A CSV already recorded as loaded is not loaded again
    #[rstest]
    fn test_should_load_csv_tracks_completed_loads(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");
        let dir = ctx
            .get_graph_import_dir()
            .expect("failed to get the import dir");
        ensure_dir_exists(&dir).expect("failed to make the import dir");

        assert!(
            db.should_load_csv(FRAMEWORK_SOURCE, CSV::Classes)
                .expect("should_load_csv failed"),
            "nothing is loaded yet"
        );

        let classes = write_csv(&dir, "classes.csv", "Lcom/a/A;,1\n");
        load(&db, ctx, &classes, FRAMEWORK_SOURCE, CSV::Classes);

        assert!(
            !db.should_load_csv(FRAMEWORK_SOURCE, CSV::Classes)
                .expect("should_load_csv failed"),
            "the load was recorded"
        );
        assert!(
            db.should_load_csv(FRAMEWORK_SOURCE, CSV::Methods)
                .expect("should_load_csv failed"),
            "a different csv for the same source is untouched"
        );
    }

    /// Every CSV kind resolves its names to the right ids
    #[rstest]
    fn test_ingest_resolves_every_csv_kind(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");
        let dir = import_dir(ctx);
        add_source(&db, SOURCE);

        load_all(&db, ctx, &dir);

        let classes = db.get_classes_for(SOURCE).expect("get_classes_for failed");
        assert_eq!(
            sorted(classes.iter().map(|it| it.get_smali_name().to_string())),
            vec!["Lz/A;", "Lz/B;", "Lz/Iface;"]
        );

        let methods = db.get_methods_for(SOURCE).expect("get_methods_for failed");
        assert_eq!(
            sorted(methods.iter().map(|it| it.name.clone())),
            vec!["alpha", "beta", "gamma", "missing"],
            "the callee this source never declares is added to it"
        );

        let child = ClassName::from("Lz/B;");
        let parents = db
            .find_parent_classes_of(&child, SOURCE)
            .expect("find_parent_classes_of failed");
        assert_eq!(
            sorted(
                parents
                    .iter()
                    .map(|it| it.name.get_smali_name().to_string())
            ),
            vec!["Lz/A;"]
        );

        let class = ClassName::from("Lz/A;");
        let ifaces = db
            .find_interfaces_of(&ClassSearch::new(&class, Some(SOURCE)))
            .expect("find_interfaces_of failed");
        assert_eq!(
            sorted(ifaces.iter().map(|it| it.name.get_smali_name().to_string())),
            vec!["Lz/Iface;"]
        );

        let alpha = method_id(&db, SOURCE, "Lz/A;", "alpha");

        let strings = db
            .get_strings_for_method(alpha)
            .expect("get_strings_for_method failed");
        assert_eq!(strings, vec!["hello-ingest"]);
        assert!(db
            .get_strings_for_source(SOURCE)
            .expect("get_strings_for_source failed")
            .contains(&String::from("hello-ingest")));

        let refs = db
            .get_method_field_refs(alpha)
            .expect("get_method_field_refs failed");
        assert_eq!(refs.len(), 1, "one field access");
        assert_eq!(refs[0].field.name, "field1");
        assert_eq!(refs[0].op, FieldAccessOp::Write);

        let callees = sorted(
            db.find_outgoing_calls(
                &MethodSearch::from(
                    MethodSearchParams::new(Some("alpha"), Some(&class), None).unwrap(),
                )
                .with_source(SOURCE),
                1,
            )
            .expect("find_outgoing_calls failed")
            .into_iter()
            .filter_map(|path| path.path.last().map(|it| it.name.clone())),
        );
        assert_eq!(callees, vec!["gamma", "missing"]);
    }

    /// A callee's class that the source never declares belongs to the framework
    #[rstest]
    fn test_ingest_puts_unknown_callee_classes_in_the_framework(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");
        let dir = import_dir(ctx);
        add_source(&db, SOURCE);

        load_all(&db, ctx, &dir);

        let owned = db.get_classes_for(SOURCE).expect("get_classes_for failed");
        assert!(
            !owned.iter().any(|it| it.get_smali_name() == "Lz/Absent;"),
            "the source does not gain a class it never declared"
        );

        let framework = db
            .get_classes_for(FRAMEWORK_SOURCE)
            .expect("get_classes_for failed");
        assert!(
            framework
                .iter()
                .any(|it| it.get_smali_name() == "Lz/Absent;"),
            "the unknown callee class lands in the framework"
        );
    }

    /// A load must not leave `foreign_keys=OFF` behind on the connection it borrowed
    ///
    /// r2d2 only customises a connection when it opens it, so a pragma the load changes
    /// outlives the load unless it is put back.
    #[rstest]
    fn test_load_csv_restores_foreign_keys(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");
        let dir = import_dir(ctx);
        add_source(&db, SOURCE);

        let classes = write_csv(&dir, "classes.csv", "Lz/A;,1\n");
        load(&db, ctx, &classes, SOURCE, CSV::Classes);

        let enforced = db.query(|c| {
            diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>("(SELECT 1)"))
                .get_result::<i32>(c)
        });
        assert!(enforced.is_ok(), "the connection still works");

        // A row pointing at a class id that cannot exist is only rejected with foreign key
        // enforcement on
        let res = db.write(|c| -> Result<()> {
            insert_into(classes::table)
                .values((
                    classes::name.eq("Lz/Orphan;"),
                    classes::source.eq(SourceId::new(9999)),
                ))
                .execute(c)?;
            Ok(())
        });
        assert!(
            matches!(res, Err(Error::ForeignKeyViolation(_))),
            "expected a foreign key violation, got {:?}",
            res
        );
    }

    /// A load that fails part way through leaves nothing behind and can be retried
    ///
    /// Each CSV is one transaction, which is what makes the recorded load status usable for
    /// restarting an import that died in the middle.
    #[rstest]
    fn test_failed_load_rolls_back_and_stays_retryable(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");
        let dir = import_dir(ctx);
        add_source(&db, SOURCE);

        // The first row is well formed, the second is missing its access flags
        let classes = write_csv(&dir, "classes.csv", "Lz/Rollback;,1\nLz/Truncated;\n");
        let res = db.load_csv(ctx, classes.to_str().unwrap(), SOURCE, CSV::Classes);
        assert!(
            res.is_err(),
            "a malformed row fails the load, got {:?}",
            res
        );

        let classes = db.get_classes_for(SOURCE).expect("get_classes_for failed");
        assert!(
            classes.is_empty(),
            "the row before the bad one is rolled back, found {:?}",
            classes
        );
        assert!(
            db.should_load_csv(SOURCE, CSV::Classes)
                .expect("should_load_csv failed"),
            "the failed load is not recorded, so an import can retry it"
        );

        // And the retry, against a good file, works on a connection the failed load used
        let classes = write_csv(&dir, "classes.csv", "Lz/Rollback;,1\n");
        load(&db, ctx, &classes, SOURCE, CSV::Classes);
        assert_eq!(
            sorted(
                db.get_classes_for(SOURCE)
                    .expect("get_classes_for failed")
                    .iter()
                    .map(|it| it.get_smali_name().to_string())
            ),
            vec!["Lz/Rollback;"]
        );
    }

    fn built_at(db: &GraphSqliteDatabase) -> i64 {
        db.query(|c| {
            _metadata::table
                .select(_metadata::built_at)
                .get_result::<i64>(c)
        })
        .expect("failed to read built_at")
    }

    fn last_delete(db: &GraphSqliteDatabase) -> Option<i64> {
        db.query(|c| {
            diesel::select(sql::<Nullable<BigInt>>(
                "(SELECT last_delete FROM _metadata)",
            ))
            .get_result::<Option<i64>>(c)
        })
        .expect("failed to read last_delete")
    }

    /// Finishing a build stamps `built_at`, which is what an artifact holding ids checks
    #[rstest]
    fn test_finalize_stamps_built_at(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");

        // Set it back rather than comparing against the migration's value: unixepoch has one
        // second of resolution, so a fresh database and a build in the same second are equal
        db.write(|c| -> Result<()> {
            sql_query("UPDATE _metadata SET built_at = 0").execute(c)?;
            Ok(())
        })
        .expect("failed to reset built_at");
        assert_eq!(built_at(&db), 0);

        db.finalize(ctx).expect("finalize failed");
        assert!(built_at(&db) > 0, "the build stamped the graph");
    }

    /// A delete is recorded permanently, because it can leave an artifact's ids dangling and
    /// a later import can reuse them
    #[rstest]
    fn test_remove_source_stamps_last_delete(tmp_context: TestContext) {
        let ctx = &tmp_context;
        let db = GraphSqliteDatabase::new(ctx).expect("failed to open the graph database");
        add_source(&db, SOURCE);

        assert_eq!(last_delete(&db), None, "nothing has been deleted yet");

        db.remove_source(SOURCE).expect("remove_source failed");

        assert!(
            last_delete(&db).is_some_and(|it| it > 0),
            "the delete is on the record"
        );
    }
}
