use std::collections::HashSet;
use std::iter::repeat;

use diesel::connection::SimpleConnection;
use diesel::sql_query;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations};
use smalisa::AccessFlag;

use super::schema::*;
use crate::db::common::DBThread;
use crate::db::common::*;
use crate::db::graph::models::Source;
use crate::db::graph::models::{
    ClassSearch, MethodCallPath, MethodSearch, MethodSearchParams, MethodSpec,
};
use crate::db::graph::{ClassSpec, GraphDatabase};
use crate::utils::ClassName;
use crate::Context;
use diesel::prelude::*;

pub static GRAPH_DATABASE_FILE_NAME: &'static str = "graph.db";
const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/graph_migrations/");

#[cfg(test)]
const TEST_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/test_graph_migrations/");

pub struct GraphSqliteDatabase {
    db_thread: DBThread,
}

impl GraphSqliteDatabase {
    pub fn new(ctx: &dyn Context) -> Result<Self> {
        let db = Self {
            db_thread: DBThread::new(
                ctx,
                GRAPH_DATABASE_FILE_NAME,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        };

        db.with_connection(|c| c.batch_execute("PRAGMA journal_mode=WAL"))?;
        Ok(db)
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
    pub(super) fn with_connection<F, R>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce(&mut SqliteConnection) -> R + Send,
    {
        self.db_thread.with_connection(f)
    }

    #[allow(unused)]
    #[inline]
    pub(super) fn transaction<F, T, E>(&self, f: F) -> std::result::Result<T, E>
    where
        T: Send,
        E: From<diesel::result::Error> + Send,
        F: FnOnce(&mut SqliteConnection) -> std::result::Result<T, E> + Send,
    {
        self.db_thread.transaction(f)
    }

    #[allow(unused)]
    pub(super) fn get_source_id(&self, source: &str) -> Result<i32> {
        Ok(self.with_connection(|c| {
            query!(sources::table
                .filter(sources::name.eq(source))
                .select(sources::id)
                .limit(1))
            .get_result::<i32>(c)
        })?)
    }

    impl_get_all!(get_sources, Source, sources);
    impl_delete_by!(delete_source_by_name, &str, sources, name.eq);

    #[inline]
    fn get_method_ids(conn: &mut SqliteConnection, search: &MethodSearch) -> Result<Vec<i32>> {
        search.param.get_sql(conn, search.source)
    }

    fn get_class_ids_sql(search: &ClassSearch) -> &'static str {
        match search.source {
            Some(_) => "SELECT c.id FROM classes AS c JOIN sources AS s ON c.source = s.id WHERE c.name = ? AND s.name = ?",
            None => "SELECT id FROM classes WHERE name = ?",
        }
    }

    fn get_calls(
        &self,
        dir: CallDirection,
        method: &MethodSearch,
        call_source: Option<&str>,
        depth: usize,
    ) -> Result<Vec<MethodCallPath>> {
        let (src, dst) = match dir {
            CallDirection::Into => ("caller", "callee"),
            CallDirection::From => ("callee", "caller"),
        };

        Ok(self.with_connection(|c| -> Result<Vec<MethodCallPath>> {
            let method_ids = Self::get_method_ids(c, method)?;

            if method_ids.is_empty() {
                return Ok(Vec::new());
            }

            let in_binds = repeat("(?)")
                .take(method_ids.len())
                .collect::<Vec<&str>>()
                .join(",");

            // Note that a UNION ALL would be much better for performance, but since the call graph
            // can be cyclic that'd potentially run us into infinite loops. Breaking cycles drops
            // useful information, so we just stick with this.

            let mut q = sql_query(format!(
                r#"WITH RECURSIVE
    search_methods(search_method_id) AS (VALUES {in_binds}),
    calls_to(methodid, distance, path) AS (
        SELECT search_method_id, 0, json_array(search_method_id) FROM search_methods
        UNION
        SELECT
            c.{src},
            ct.distance + 1,
            json_insert(ct.path, '$[#]', c.{src})
        FROM calls AS c
        JOIN calls_to AS ct
            ON ct.methodid = c.{dst}
        WHERE ct.distance < ?
        ORDER BY 2 DESC
    ),
    method_calls(source, class, name, args, ret, access_flags, idx) AS (
        SELECT s.name, c.name, m.name, m.args, m.ret, m.access_flags, CAST(p.key AS INTEGER)
        FROM calls_to AS ct
        JOIN json_each(ct.path) AS p
        JOIN methods AS m
            ON m.id = CAST(p.value AS INTEGER)
        JOIN sources AS s
            ON s.id = m.source
        JOIN classes AS c
            ON c.id = m.class
        WHERE ct.distance > 0
    )
SELECT * from method_calls;"#,
            ))
            .into_boxed();

            for mid in method_ids {
                q = q.bind::<Integer, _>(mid);
            }

            let int_depth: i32 = depth
                .try_into()
                .map_err(|_| Error::Generic(format!("invalid depth")))?;

            q = q.bind::<Integer, _>(int_depth);

            let rows: Vec<MethodCallRow> = query!(q).get_results(c)?;
            let it = PathRowIterator::new(rows.into_iter());

            let res = it.collect::<MethodSpec>(true).into_iter();

            Ok(match call_source {
                None => res.map(MethodCallPath::from).collect(),
                Some(src) => res
                    .filter_map(|it| {
                        if it.first()?.source == src {
                            Some(it)
                        } else {
                            None
                        }
                    })
                    .map(MethodCallPath::from)
                    .collect(),
            })
        })?)
    }
}

enum CallDirection {
    From,
    Into,
}

impl<'a> MethodSearchParams<'a> {
    fn get_sql(&self, conn: &mut SqliteConnection, source: Option<&str>) -> Result<Vec<i32>> {
        match self {
            Self::ByName { name } => self.sql_by_name(conn, name, source),
            Self::ByClass { class } => self.sql_by_class(conn, &class.get_smali_name(), source),
            Self::ByClassAndName { class, name } => {
                self.sql_by_class_and_name(conn, &class.get_smali_name(), name, source)
            }
            Self::ByNameAndSignature { name, signature } => {
                self.sql_by_name_and_sig(conn, name, signature, source)
            }
            Self::ByFullSpec {
                class,
                name,
                signature,
            } => self.sql_by_full_spec(conn, &class.get_smali_name(), name, signature, source),
        }
    }

    fn sql_by_full_spec(
        &self,
        conn: &mut SqliteConnection,
        class: &str,
        name: &str,
        sig: &str,
        source: Option<&str>,
    ) -> Result<Vec<i32>> {
        Ok(match source {
            Some(v) => query!(methods::table
                .inner_join(sources::table)
                .inner_join(classes::table)
                .filter(sources::name.eq(v))
                .filter(classes::name.eq(class))
                .filter(methods::args.eq(sig))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
            None => query!(methods::table
                .inner_join(classes::table)
                .filter(methods::args.eq(sig))
                .filter(classes::name.eq(class))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
        }?)
    }

    fn sql_by_name_and_sig(
        &self,
        conn: &mut SqliteConnection,
        name: &str,
        sig: &str,
        source: Option<&str>,
    ) -> Result<Vec<i32>> {
        Ok(match source {
            Some(v) => query!(methods::table
                .inner_join(sources::table)
                .filter(sources::name.eq(v))
                .filter(methods::args.eq(sig))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
            None => query!(methods::table
                .filter(methods::args.eq(sig))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
        }?)
    }

    fn sql_by_class_and_name(
        &self,
        conn: &mut SqliteConnection,
        class: &str,
        name: &str,
        source: Option<&str>,
    ) -> Result<Vec<i32>> {
        Ok(match source {
            Some(v) => query!(methods::table
                .inner_join(sources::table)
                .inner_join(classes::table)
                .filter(sources::name.eq(v))
                .filter(classes::name.eq(class))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
            None => query!(methods::table
                .inner_join(classes::table)
                .filter(classes::name.eq(class))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
        }?)
    }

    fn sql_by_class(
        &self,
        conn: &mut SqliteConnection,
        class: &str,
        source: Option<&str>,
    ) -> Result<Vec<i32>> {
        Ok(match source {
            Some(v) => query!(methods::table
                .inner_join(sources::table)
                .inner_join(classes::table)
                .filter(sources::name.eq(v))
                .filter(classes::name.eq(class))
                .select(methods::id))
            .load::<i32>(conn),
            None => query!(methods::table
                .inner_join(classes::table)
                .filter(classes::name.eq(class))
                .select(methods::id))
            .load::<i32>(conn),
        }?)
    }

    fn sql_by_name(
        &self,
        conn: &mut SqliteConnection,
        name: &str,
        source: Option<&str>,
    ) -> Result<Vec<i32>> {
        Ok(match source {
            Some(v) => query!(methods::table
                .inner_join(sources::table)
                .filter(sources::name.eq(v))
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
            None => query!(methods::table
                .filter(methods::name.eq(name))
                .select(methods::id))
            .load::<i32>(conn),
        }?)
    }
}

impl GraphDatabase for GraphSqliteDatabase {
    fn find_callers(
        &self,
        method: &MethodSearch,
        call_source: Option<&str>,
        depth: usize,
    ) -> Result<Vec<MethodCallPath>> {
        self.get_calls(CallDirection::Into, method, call_source, depth)
    }
    fn wipe(&self, ctx: &dyn Context) -> Result<()> {
        let path = ctx.get_sqlite_dir()?.join(GRAPH_DATABASE_FILE_NAME);
        std::fs::remove_file(&path)?;
        Ok(())
    }
    fn remove_source(&self, source: &str) -> Result<()> {
        Ok(self.delete_source_by_name(source)?)
    }

    fn get_all_sources(&self) -> Result<HashSet<String>> {
        let sources = self.get_sources()?;
        let mut m = HashSet::with_capacity(sources.len());
        m.extend(sources.into_iter().map(|it| it.name));
        Ok(m)
    }

    fn get_classes_for(&self, source: &str) -> Result<Vec<ClassName>> {
        self.with_connection(|c| -> Result<Vec<ClassName>> {
            Ok(query!(classes::table
                .inner_join(sources::table)
                .filter(sources::name.eq(source))
                .select(classes::name))
            .load::<ClassName>(c)?)
        })
    }

    fn get_methods_for(&self, source: &str) -> Result<Vec<MethodSpec>> {
        self.with_connection(|c| -> Result<Vec<MethodSpec>> {
            let rows = query!(methods::table
                .inner_join(sources::table)
                .inner_join(classes::table)
                .filter(sources::name.eq(source))
                .select((
                    classes::name,
                    methods::name,
                    methods::args,
                    methods::ret,
                    methods::access_flags,
                    sources::name,
                )))
            .load::<(String, String, String, String, i64, String)>(c)?;
            Ok(rows
                .into_iter()
                .map(|(class, name, signature, ret, flags, source)| MethodSpec {
                    class: class.into(),
                    name,
                    signature,
                    ret,
                    access_flags: AccessFlag::from_bits_truncate(flags as u64),
                    source,
                })
                .collect())
        })
    }

    fn find_outgoing_calls(
        &self,
        from: &MethodSearch,
        depth: usize,
    ) -> Result<Vec<MethodCallPath>> {
        self.get_calls(CallDirection::From, from, None, depth)
    }

    fn find_child_classes_of(
        &self,
        parent: &ClassSearch,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>> {
        self.with_connection(|c| -> Result<Vec<ClassSpec>> {
            let get_class_ids_sql = Self::get_class_ids_sql(parent);

            let src_where = if source.is_some() {
                "AND s.name = ?"
            } else {
                ""
            };

            // Since we're expecting well formed data, the UNION ALL here shouldn't be a problem
            // since there should never be a cyclic inheritance graph in Java. At least I think. TBH
            // I'm just asserting that.
            let mut q = sql_query(format!(
                r#"WITH RECURSIVE
    search_classes(search_class_id) AS ({get_class_ids_sql}),

    child_classes(classid, distance) AS (
        SELECT search_class_id, 0 FROM search_classes
        UNION ALL
        SELECT
           s.child, 
           cc.distance + 1
        FROM supers AS s
        JOIN child_classes AS cc
            ON cc.classid = s.parent
    ),
    class_specs(source, name, access_flags) AS (
        SELECT s.name, c.name, c.access_flags
        FROM child_classes AS cc
        JOIN classes AS c
            ON c.id = cc.classid
        JOIN sources AS s
            on s.id = c.source
        WHERE cc.distance > 0 {src_where}
    )
SELECT DISTINCT source, name, access_flags from class_specs
            "#
            ))
            .into_boxed();

            let search_name = parent.class.get_smali_name();

            q = q.bind::<Text, _>(search_name.into_owned());

            if let Some(s) = parent.source {
                q = q.bind::<Text, _>(String::from(s));
            }

            if let Some(s) = source {
                q = q.bind::<Text, _>(String::from(s));
            }

            let rows: Vec<ChildClassRow> = query!(q).get_results(c)?;

            Ok(rows.into_iter().map(ClassSpec::from).collect())
        })
    }

    fn find_classes_implementing(
        &self,
        iface: &ClassSearch,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>> {
        self.with_connection(|c| {
            let get_class_ids_sql = Self::get_class_ids_sql(iface);

            let src_where = if source.is_some() {
                "WHERE s.name = ?"
            } else {
                ""
            };

            // UNION ALL here is probably safe for the same reason as `supers`
            let mut q = sql_query(format!(
                r#" WITH RECURSIVE
    search_classes(search_class_id) AS ({get_class_ids_sql}),

    impl_classes(classid, distance) AS (
        SELECT search_class_id, 0 FROM search_classes
        UNION ALL
        SELECT
           s.class, 
           cc.distance + 1
        FROM interfaces AS s
        JOIN impl_classes AS cc
            ON cc.classid = s.interface
    ),
    child_classes(classid, distance) AS (
        SELECT impl.classid, 0 FROM impl_classes AS impl WHERE impl.distance > 0
        UNION ALL
        SELECT
           s.child, 
           cc.distance + 1
        FROM supers AS s
        JOIN child_classes AS cc
            ON cc.classid = s.parent
    ),


    all_classes(classid, distance) AS (
        SELECT * FROM child_classes WHERE child_classes.distance > 0
        UNION 
        SELECT * FROM impl_classes WHERE impl_classes.distance > 0
    ),

    class_specs(source, name, access_flags) AS (
        SELECT s.name, c.name, c.access_flags
        FROM all_classes AS ac
        JOIN classes AS c
            ON c.id = ac.classid
        JOIN sources AS s
            on s.id = c.source
        {src_where}
    )

SELECT DISTINCT source, name, access_flags from class_specs"#
            ))
            .into_boxed();

            let search_name = iface.class.get_smali_name();

            q = q.bind::<Text, _>(search_name.into_owned());

            if let Some(s) = iface.source {
                q = q.bind::<Text, _>(String::from(s));
            }

            if let Some(s) = source {
                q = q.bind::<Text, _>(String::from(s));
            }

            let rows: Vec<ChildClassRow> = query!(q).get_results(c)?;

            Ok(rows.into_iter().map(ClassSpec::from).collect())
        })
    }
}

#[derive(QueryableByName, Debug)]
struct MethodCallRow {
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    class: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    args: String,
    #[diesel(sql_type = Text)]
    ret: String,
    #[diesel(sql_type = BigInt)]
    access_flags: i64,
    #[diesel(sql_type = Integer)]
    idx: i32,
}

impl From<MethodCallRow> for MethodSpec {
    fn from(value: MethodCallRow) -> Self {
        Self {
            class: ClassName::from(value.class),
            ret: value.ret,
            name: value.name,
            signature: value.args,
            source: value.source,
            access_flags: AccessFlag::from_bits_truncate(value.access_flags as u64),
        }
    }
}

trait Indexable {
    fn idx(&self) -> i32;
}

impl Indexable for MethodCallRow {
    fn idx(&self) -> i32 {
        self.idx
    }
}

#[derive(QueryableByName, Debug)]
struct ChildClassRow {
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Integer)]
    access_flags: i32,
}

impl From<ChildClassRow> for ClassSpec {
    fn from(value: ChildClassRow) -> Self {
        Self {
            name: ClassName::from(value.name),
            access_flags: AccessFlag::from_bits_truncate(value.access_flags as u64),
            source: value.source,
        }
    }
}

struct PathRowIterator<T, I>
where
    T: Indexable,
    I: Iterator<Item = T>,
{
    it: I,
    first: Option<T>,
}

impl<T, I> PathRowIterator<T, I>
where
    T: Indexable,
    I: Iterator<Item = T>,
{
    fn new(mut it: I) -> Self {
        let first = it.next();
        Self { it, first }
    }

    fn next_in_seq(&mut self) -> Option<T> {
        if let Some(first) = self.first.take() {
            return Some(first);
        }

        let next = self.it.next()?;
        if next.idx() == 0 {
            self.first = Some(next);
            None
        } else {
            Some(next)
        }
    }

    fn collect<U>(mut self, reverse: bool) -> Vec<Vec<U>>
    where
        U: From<T>,
    {
        let mut results = Vec::new();
        // This first call always hits the cached row.idx == 0 value, so if it
        // returns None we're done

        while let Some(row) = self.next_in_seq() {
            let mut path: Vec<U> = Vec::new();
            path.push(row.into());

            // When this goes to None, we're just done with the sequence and caching
            // the row.idx == 0 value
            while let Some(row) = self.next_in_seq() {
                if reverse {
                    path.insert(0, row.into());
                } else {
                    path.push(row.into());
                }
            }
            results.push(path);
        }

        results
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
            dir.join(GRAPH_DATABASE_FILE_NAME).to_string_lossy()
        )
    }

    fn db_test(context: &dyn Context, func: impl FnOnce(GraphSqliteDatabase)) {
        let url = get_db_url(&context);
        let db = GraphSqliteDatabase::new_from_url(&url).expect("failed to get database");
        let res = panic::catch_unwind(AssertUnwindSafe(|| func(db)));
        cleanup_database(&url);
        match res {
            Err(e) => panic::resume_unwind(e),
            _ => {}
        }
    }

    #[rstest]
    fn test_find_classes_implementing(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            macro_rules! get_impl {
                    ($name:expr, $parentsrc:expr, [$({ $($field:ident: $value:expr),+ }),*]) => {
                        get_impl!($name, $parentsrc, None, [$({ $($field: $value),+ }),*])
                    };

                    ($name:expr, $parentsrc:expr, $src:expr, [$({ $($field:ident: $value:expr),+ }),*]) => {
                        let name = ClassName::from($name);
                        let search = ClassSearch::new(&name, $parentsrc);
                        let expected: Vec<ClassSpec> = vec![$(ClassSpec { $($field: $value.into()),+ }),*];
                        let classes: Vec<ClassSpec> =
                            db.find_classes_implementing(&search, $src).expect("find_classes_implementing call failed");
                        assert_eq!(classes, expected);
                    };
                }

            get_impl!("Lae/ae;", Some("B"), [
                {name: "Lax/ax;", source: "C", access_flags: AccessFlag::PUBLIC},
                {name: "Laz/az;", source: "E", access_flags: AccessFlag::PUBLIC},
                {name: "Lca/ca;", source: "framework", access_flags: AccessFlag::PUBLIC}
            ]);

            get_impl!("Lae/ae;", Some("D"), []);

            get_impl!("Lae/ae;", None, [
                {name: "Lam/am;", source: "B", access_flags: AccessFlag::PUBLIC},
                {name: "Lax/ax;", source: "C", access_flags: AccessFlag::PUBLIC},
                {name: "Laz/az;", source: "E", access_flags: AccessFlag::PUBLIC},
                {name: "Lbz/bz;", source: "D", access_flags: AccessFlag::PUBLIC},
                {name: "Lca/ca;", source: "framework", access_flags: AccessFlag::PUBLIC},
                {name: "Lcb/cb;", source: "C", access_flags: AccessFlag::PUBLIC},
                {name: "Lcc/cc;", source: "C", access_flags: AccessFlag::PUBLIC},
                {name: "Lcx/cx;", source: "B", access_flags: AccessFlag::PUBLIC},
                {name: "Ldd/dd;", source: "B", access_flags: AccessFlag::PUBLIC}
            ]);

            get_impl!("Lae/ae;", Some("B"), Some("framework"), [
                {name: "Lca/ca;", source: "framework", access_flags: AccessFlag::PUBLIC}
            ]);
        });
    }

    #[rstest]
    fn test_find_child_classes_of(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            macro_rules! get_children {
                    ($name:expr, $parentsrc:expr, [$({ $($field:ident: $value:expr),+ }),*]) => {
                        get_children!($name, $parentsrc, None, [$({ $($field: $value),+ }),*])
                    };

                    ($name:expr, $parentsrc:expr, $src:expr, [$({ $($field:ident: $value:expr),+ }),*]) => {
                        let name = ClassName::from($name);
                        let search = ClassSearch::new(&name, $parentsrc);
                        let expected: Vec<ClassSpec> = vec![$(ClassSpec { $($field: $value.into()),+ }),*];
                        let classes: Vec<ClassSpec> =
                            db.find_child_classes_of(&search, $src).expect("find_child_classes_of call failed");
                        assert_eq!(classes, expected);
                    };
                }

            get_children!("Lbb/bb;", None, [
                { name: "Lan/an;", access_flags: AccessFlag::PUBLIC, source: "C" },
                { name: "Lan/an;", access_flags: AccessFlag::PUBLIC, source: "D" },
                { name: "Lcz/cz;", access_flags: AccessFlag::PUBLIC, source: "E" },
                { name: "Ldc/dc;", access_flags: AccessFlag::PUBLIC, source: "B" },
                { name: "Lde/de;", access_flags: AccessFlag::PUBLIC, source: "D" }
            ]);

            get_children!("Lbb/bb;", Some("framework"), [
                { name: "Lan/an;", access_flags: AccessFlag::PUBLIC, source: "C" }
            ]);

            get_children!("Lbb/bb;", Some("B"), [
                { name: "Lan/an;", access_flags: AccessFlag::PUBLIC, source: "D" },
                { name: "Lcz/cz;", access_flags: AccessFlag::PUBLIC, source: "E" },
                { name: "Ldc/dc;", access_flags: AccessFlag::PUBLIC, source: "B" },
                { name: "Lde/de;", access_flags: AccessFlag::PUBLIC, source: "D" }
            ]);

            get_children!("Lbb/bb;", Some("D"), []);

            get_children!("Lbb/bb;", Some("B"), Some("B"), [
                { name: "Ldc/dc;", access_flags: AccessFlag::PUBLIC, source: "B" }
            ]);
        });
    }

    #[rstest]
    fn test_get_classes_for(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let results = db
                .get_classes_for("framework")
                .expect("failed to get classes");
            assert_eq!(results.len(), 25);
        });
    }

    #[rstest]
    fn test_get_methods_for(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let results = db
                .get_methods_for("framework")
                .expect("failed to get methods");
            assert_eq!(results.len(), 75);
        });
    }

    #[rstest]
    fn test_get_callers(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let class = ClassName::from("Lbl/bl;");
            let method = MethodSearch::new(
                MethodSearchParams::ByFullSpec {
                    class: &class,
                    name: "by",
                    signature: "JLjava/lang/String;J",
                },
                None,
            );

            macro_rules! path {
                ($({ $($name:ident: $val:expr),+ }),*) => {{

                    let path = vec![$(
                            MethodSpec {
                        $(
                                $name: $val.into()
                        ),+,
                                access_flags: AccessFlag::PUBLIC,
                            }
                    ),*];
                    MethodCallPath {
                        path
                    }
                }};
            }

            let callers = db.find_callers(&method, None, 3).expect("find_callers");
            assert_eq!(
                callers,
                vec![
                    path!(
                        {class: "ax.ax", name: "ds", signature: "FIJ", source: "C", ret: "Ljava/lang/String;"},
                        {class: "bl.bl", name: "by", signature: "JLjava/lang/String;J", source: "E", ret: "Landroid/os/IBinder;"}
                    ),
                    path!(
                        {class: "bs.bs", name: "fe", signature: "J", source: "framework", ret: "C"},
                        {class: "bl.bl", name: "by", signature: "JLjava/lang/String;J", source: "E", ret: "Landroid/os/IBinder;"}
                    ),
                    path!(
                        {class: "al.al", name: "fi", signature: "IZLjava/lang/String;", source: "C", ret: "C"},
                        {class: "bs.bs", name: "fe", signature: "J", source: "framework", ret: "C"},
                        {class: "bl.bl", name: "by", signature: "JLjava/lang/String;J", source: "E", ret: "Landroid/os/IBinder;"}
                    )
                ]
            );

            let callers = db
                .find_callers(&method, Some("C"), 3)
                .expect("find_callers");
            assert_eq!(
                callers,
                vec![
                    path!(
                        {class: "ax.ax", name: "ds", signature: "FIJ", source: "C", ret: "Ljava/lang/String;"},
                        {class: "bl.bl", name: "by", signature: "JLjava/lang/String;J", source: "E", ret: "Landroid/os/IBinder;"}
                    ),
                    path!(
                        {class: "al.al", name: "fi", signature: "IZLjava/lang/String;", source: "C", ret: "C"},
                        {class: "bs.bs", name: "fe", signature: "J", source: "framework", ret: "C"},
                        {class: "bl.bl", name: "by", signature: "JLjava/lang/String;J", source: "E", ret: "Landroid/os/IBinder;"}
                    )
                ]
            );
        });
    }

    #[rstest]
    fn test_get_method_ids(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            macro_rules! get_mids {
                ($sel:ident { $($name:ident: $val:expr),+ }, [$($expected:expr),*]) => {
                    get_mids!($sel { $($name: $val),+ }, [$($expected),*], None)
                };

                ($sel:ident { $($name:ident: $val:expr),+ }, [$($expected:expr),*], $src:expr) => {
                    let mids: Vec<i32> = db.with_connection(|c| {
                        GraphSqliteDatabase::get_method_ids(c, &MethodSearch::new(MethodSearchParams::$sel { $($name: $val),+ }, $src))

                    }).expect("get_method_ids call failed");
                    for id in [$($expected as i32),*] {
                        assert!(mids.contains(&id), "expected to find {id} in method ids but it wasn't in {mids:?}");
                    }
                };

            }

            get_mids!(ByName { name: "dq" }, [189, 190]);
            get_mids!(ByName { name: "dq" }, [190], Some("C"));
            get_mids!(ByName { name: "dq" }, [], Some("framework"));

            get_mids!(
                ByClass {
                    class: &ClassName::from("Laj/aj;")
                },
                [32, 110, 186, 31, 109, 185]
            );

            get_mids!(
                ByClass {
                    class: &ClassName::from("Laj/aj;")
                },
                [32, 110, 186],
                Some("B")
            );

            get_mids!(
                ByClass {
                    class: &ClassName::from("Laj/aj;")
                },
                [],
                Some("D")
            );

            get_mids!(
                ByClassAndName {
                    class: &ClassName::from("Laj/aj;"),
                    name: "ap"
                },
                [32, 31]
            );

            get_mids!(
                ByClassAndName {
                    class: &ClassName::from("Laj/aj;"),
                    name: "ap"
                },
                [31],
                Some("C")
            );

            get_mids!(
                ByClassAndName {
                    class: &ClassName::from("Laj/aj;"),
                    name: "ap"
                },
                [],
                Some("D")
            );

            get_mids!(
                ByFullSpec {
                    class: &ClassName::from("Laj/aj;"),
                    name: "ap",
                    signature: "DLjava/lang/String;J"
                },
                [32, 31]
            );

            get_mids!(
                ByFullSpec {
                    class: &ClassName::from("Laj/aj;"),
                    name: "ap",
                    signature: "DLjava/lang/String;J"
                },
                [32],
                Some("B")
            );

            get_mids!(
                ByFullSpec {
                    class: &ClassName::from("Laj/aj;"),
                    name: "ap",
                    signature: "DLjava/lang/String;J"
                },
                [],
                Some("D")
            );

            get_mids!(
                ByNameAndSignature {
                    name: "aa",
                    signature: ""
                },
                [1, 2]
            );

            get_mids!(
                ByNameAndSignature {
                    name: "aa",
                    signature: ""
                },
                [1],
                Some("B")
            );

            get_mids!(
                ByNameAndSignature {
                    name: "aa",
                    signature: ""
                },
                [],
                Some("D")
            );
        });
    }
}
