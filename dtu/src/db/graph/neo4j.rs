use neo4rs::{self, Graph, Node, Path, Query, Row};
use smalisa::AccessFlag;
use tokio::runtime::Runtime;

use super::db::LoadCSVKind;
use super::models::{ClassCallPath, ClassMeta, MethodCallSearch, MethodMeta};
use super::{Error, GraphDatabase, GraphDatabaseInternal, Result};
use crate::db::graph::common::AddDirTask;
use crate::db::graph::db::{AddDirectoryOptions, SetupEvent, FRAMEWORK_SOURCE};
use crate::tasks::{EventMonitor, TaskCancelCheck};
use crate::utils::ClassName;
use crate::Context;

/// A [GraphDatabase] implementation backed by Neo4j
pub struct Neo4jDatabase {
    rt: Runtime,
    graph: Graph,
}

// Defining this as a macro because the `BoltType` type isn't exported so
// I can't write a wrapper function.
macro_rules! get_field {
    ($row:ident, $field:expr) => {{
        $row.get($field).or_else(|_| {
            ::log::trace!("{:?}", $row);
            Err(Error::MissingField(String::from($field)))
        })
    }};
}

macro_rules! maybe_get_field {
    ($row:ident, $field:expr) => {{
        $row.get($field).ok()
    }};
}

/// TODO: This was just randomly chosen, optimize I guess
const TRANSACTION_SIZE: i32 = 1000;

impl TryFrom<&Node> for ClassMeta {
    type Error = Error;

    fn try_from(value: &Node) -> std::result::Result<Self, Self::Error> {
        let name = get_field!(value, "name")?;
        let source = get_field!(value, "source")?;
        let value: i64 = get_field!(value, "access_flags")?;
        let access_flags = AccessFlag::from_bits_truncate(value as u64);
        Ok(ClassMeta {
            name: ClassName::new(name),
            access_flags,
            source,
        })
    }
}

impl TryFrom<&Node> for MethodMeta {
    type Error = Error;
    fn try_from(node: &Node) -> std::result::Result<Self, Self::Error> {
        let class = get_field!(node, "class")?;
        let name = get_field!(node, "name")?;
        let signature = get_field!(node, "sig")?;
        let ret = maybe_get_field!(node, "ret");
        Ok(Self {
            class: ClassName::new(class),
            name,
            signature,
            ret,
        })
    }
}

fn to_node(row: &Row, key: &str) -> Result<Node> {
    let as_node: Node = match row.get(key) {
        Ok(v) => v,
        Err(e) => return Err(Error::Generic(format!("deserializing node: {}", e))),
    };
    Ok(as_node)
}

impl<'a> MethodCallSearch<'a> {
    fn get_selectors(
        &self,
        is_target: bool,
        class: Option<&ClassName>,
        method: Option<&str>,
        sig: Option<&str>,
    ) -> String {
        let prefix = if is_target { "to" } else { "from" };
        let mut s = String::new();

        if class.is_some() {
            s.push_str(&format!("class: ${}_class", prefix));
        }

        if method.is_some() {
            s.push_str(&format!("name: ${}_name", prefix));
        }

        if sig.is_some() {
            s.push_str(&format!("sig: ${}_sig", prefix));
        }

        s
    }

    fn add_selectors_to_query(
        &self,
        mut query: Query,
        is_target: bool,
        class: Option<&ClassName>,
        method: Option<&str>,
        sig: Option<&str>,
    ) -> Query {
        let prefix = if is_target { "to" } else { "from" };

        query = if let Some(class) = class {
            query.param(
                &format!("{}_class", prefix),
                class.get_smali_name().as_ref(),
            )
        } else {
            query
        };

        query = if let Some(method) = method {
            query.param(&format!("{}_name", prefix), method)
        } else {
            query
        };

        query = if let Some(sig) = sig {
            query.param(&format!("{}_sig", prefix), sig)
        } else {
            query
        };

        query
    }

    fn get_source_selector(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "Method{{{}}}",
            self.get_selectors(
                false,
                self.src_class,
                self.src_method_name,
                self.src_method_sig
            )
        ));

        s
    }

    fn get_target_selector(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "Method{{{}}}",
            self.get_selectors(
                true,
                self.target_class,
                Some(self.target_method),
                Some(self.target_method_sig)
            )
        ));

        s
    }

    fn add_to_query(&self, mut q: Query) -> Query {
        q = self.add_selectors_to_query(
            q,
            false,
            self.src_class,
            self.src_method_name,
            self.src_method_sig,
        );
        self.add_selectors_to_query(
            q,
            true,
            self.target_class,
            Some(self.target_method),
            Some(self.target_method_sig),
        )
    }
}

impl GraphDatabase for Neo4jDatabase {
    fn initialize(&self) -> Result<()> {
        let mut already_init = false;
        self.execute_simple_query(
            "MATCH (d:DatabaseInitialized) RETURN count(d) LIMIT 1",
            |_row| {
                already_init = true;
                Ok(())
            },
        )?;
        if already_init {
            return Ok(());
        }
        self.create_constraints_and_indices()?;
        self.run_stmt("CREATE (:DatabaseInitialized)")?;
        Ok(())
    }

    fn add_directory(
        &self,
        ctx: &dyn Context,
        opts: AddDirectoryOptions,
        monitor: &dyn EventMonitor<SetupEvent>,
        cancel: &TaskCancelCheck,
    ) -> Result<()> {
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

    fn find_child_classes_of(
        &self,
        parent: &ClassName,
        source: Option<&str>,
    ) -> Result<Vec<ClassMeta>> {
        let cname = parent;
        let class = cname.get_smali_name();
        let query = if source.is_some() {
            r#"
MATCH (parent:Class {name: $name })-[:HAS_CHILD* { source: $source }]->(child:Class)
RETURN DISTINCT child"#
        } else {
            r#"
MATCH (parent:Class {name: $name})-[:HAS_CHILD*]->(child:Class)
RETURN DISTINCT child"#
        };

        let mut res: Vec<ClassMeta> = Vec::new();
        self.execute_query(
            &query,
            |r| {
                res.push(ClassMeta::try_from(&to_node(&r, "child")?)?);
                Ok(())
            },
            |q| {
                let q = q.param("name", class.as_ref());
                if let Some(src) = source {
                    q.param("source", src)
                } else {
                    q
                }
            },
        )?;

        Ok(res)
    }

    fn find_classes_implementing(
        &self,
        iface: &ClassName,
        source: Option<&str>,
    ) -> Result<Vec<ClassMeta>> {
        let cname = iface;
        let iface = cname.get_smali_name();

        let query = if source.is_some() {
            r#"
MATCH(iface:Class { name: $name })<-[:IMPLEMENTS*]-(impl:Class { source: $source })
RETURN impl
UNION ALL
MATCH(child:Class { source: $source })<-[:HAS_CHILD*]-(:Class)-[:IMPLEMENTS*]->(iface:Class { name: $name })
RETURN child AS impl
"#
        } else {
            r#"
MATCH(iface:Class {name: $name})<-[:IMPLEMENTS*]-(impl:Class)
RETURN impl
UNION ALL
MATCH(child:Class)<-[:HAS_CHILD*]-(:Class)-[:IMPLEMENTS*]->(iface:Class {name: $name})
RETURN child AS impl
"#
        };

        let mut res: Vec<ClassMeta> = Vec::new();
        self.execute_query(
            &query,
            |r| {
                res.push(ClassMeta::try_from(&to_node(&r, "impl")?)?);
                Ok(())
            },
            |q| {
                let q = q.param("name", iface.as_ref());
                if let Some(src) = source {
                    q.param("source", src)
                } else {
                    q
                }
            },
        )?;

        Ok(res)
    }

    fn find_callers(
        &self,
        method: &MethodCallSearch,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<ClassCallPath>> {
        let src_selector = method.get_source_selector();
        let target_selector = method.get_target_selector();
        let query = if let Some(limit) = limit {
            format!(
                "MATCH path=(source:{})-[:CALLS*1..{}]->(target:{}) RETURN path LIMIT {}",
                src_selector, depth, target_selector, limit
            )
        } else {
            format!(
                "MATCH path=(source:{})-[:CALLS*1..{}]->(target:{}) RETURN path",
                src_selector, depth, target_selector
            )
        };

        let setup_func = |q: Query| method.add_to_query(q);

        let mut into = Vec::new();
        self.execute_query(
            &query,
            |row| {
                let path: Path = get_field!(row, "path")?;

                let nodes = path.nodes();

                let start_node = nodes
                    .first()
                    .ok_or_else(|| Error::Generic("empty path".into()))?;

                let class = ClassName::new(get_field!(start_node, "class")?);

                let mut methods = Vec::new();
                for n in nodes {
                    methods.push(MethodMeta::try_from(&n)?);
                }

                into.push(ClassCallPath {
                    class,
                    path: methods,
                });
                Ok(())
            },
            setup_func,
        )?;
        Ok(into)
    }

    fn wipe(&self, _ctx: &dyn Context) -> Result<()> {
        self.wipe_database()
    }

    fn remove_source(&self, source: &str) -> Result<()> {
        log::info!("Removing {} from neo4j database", source);
        log::debug!("Deleting relations...");
        self.run_binding_stmt("CALL { MATCH ()-[r]->() WHERE r.source = $source DELETE r } IN TRANSACTIONS OF 1000 ROWS", |q| q.param("source", source))?;
        log::debug!("Deleting nodes...");
        self.run_binding_stmt(
            "CALL { MATCH (n) WHERE n.source = $source DELETE n } IN TRANSACTIONS of 1000 ROWS",
            |q| q.param("source", source),
        )?;
        log::info!("removed {} from neo4j database", source);
        Ok(())
    }
}

impl GraphDatabaseInternal for Neo4jDatabase {
    #[allow(unused_variables)]
    fn load_classes_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let query = format!(
            concat!(
            "LOAD CSV FROM 'file:///{}' AS line CALL {{ WITH line ",
            "MERGE (c:Class {{ name: line[0], access_flags: toInteger(line[1]), source: $source }}) ",
            "}} IN TRANSACTIONS OF {} ROWS"
            ),
            path,
            TRANSACTION_SIZE
        );

        self.run_binding_stmt(&query, |q| q.param("source", source))?;
        Ok(())
    }

    fn should_load_csv(&self, source: &str, csv: LoadCSVKind) -> bool {
        let query = self.get_existence_query(csv, "$source", "count");

        let mut has_any = false;

        if let Err(e) = self.execute_query(
            &query,
            |row| {
                let value: i64 = match row.get("count") {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(Error::Generic(format!(
                            "failed to get count for query {}: {}",
                            query, e
                        )))
                    }
                };

                has_any = value > 0;
                Ok(())
            },
            |q| q.param("source", source),
        ) {
            log::error!("failed to execute query {}: {}", query, e);
            return false;
        }

        !has_any
    }

    #[allow(unused_variables)]
    fn load_supers_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        // Classes can be children of classes inside the given dir or part of the framework, but we
        // don't know which from the simple analysis we did. To get around this, we'll first check
        // if the class exists in the given source via an OPTIONAL MATCH and, if so, use that one.
        // Otherwise, we'll MERGE the class with the FRAMEWORK_SOURCE and use that.

        let query = format!(
            r#"LOAD CSV FROM 'file:///{}' AS line CALL {{
WITH line

MATCH (c:Class {{ name: line[0], source: $source }})

OPTIONAL MATCH (parent:Class {{ name: line[1], source: $source }})

CALL apoc.do.when (
    parent IS NOT NULL,
    "RETURN parent AS p",
    "MERGE (new:Class {{name: name, source: '{}' }}) ON CREATE SET new.access_flags = 2 RETURN new AS p",
    {{parent: parent, name: line[1]}}
) YIELD value

WITH value.p AS parent, c

CREATE(parent)-[:HAS_CHILD {{source: $source}}]->(c)

}} IN TRANSACTIONS OF {} ROWS"#,
            path, FRAMEWORK_SOURCE, TRANSACTION_SIZE
        );
        self.run_binding_stmt(&query, |q| q.param("source", source))?;
        Ok(())
    }

    #[allow(unused_variables)]
    fn load_impls_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        // See supers for a description of this query
        let query = format!(
            r#"LOAD CSV FROM 'file:///{}' AS line CALL {{
WITH line 

MATCH (impl:Class {{ name: line[0], source: $source }})

OPTIONAL MATCH (interface:Class {{ name: line[1] }})

CALL apoc.do.when(
    interface IS NOT NULL,
    "RETURN interface AS iface",
    "MERGE (c:Class {{ name: name, source: '{}' }}) ON CREATE SET c.access_flags = 32770 RETURN c AS iface",
    {{interface: interface, name: line[1]}}
) YIELD value

WITH value.iface AS iface, impl

CREATE(impl)-[:IMPLEMENTS {{source: $source}}]->(iface)

}} IN TRANSACTIONS OF {} ROWS"#,
            path, FRAMEWORK_SOURCE, TRANSACTION_SIZE
        );
        self.run_binding_stmt(&query, |q| q.param("source", source))?;
        Ok(())
    }

    #[allow(unused_variables)]
    fn load_methods_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let query = format!(
            r#"LOAD CSV FROM 'file:///{}' AS line CALL {{
WITH line 

MATCH (owner:Class {{ name: line[0], source: $source }})

CREATE (method:Method {{
    class: line[0],
    name: line[1],
    source: $source,
    sig: coalesce(line[2], ''),
    ret: coalesce(line[3], 'V')
}})

CREATE (owner)-[:HAS_METHOD {{source: $source}}]->(method)

}} IN TRANSACTIONS OF {} ROWS"#,
            path, TRANSACTION_SIZE
        );

        self.run_binding_stmt(&query, |q| q.param("source", source))?;
        Ok(())
    }

    #[allow(unused_variables)]
    fn load_calls_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        // This is similar to the above queries with the same format. A note
        // here is that if the method doesn't exist in the framework for
        // whatever reason we lose the return type
        let query = format!(
            r#"LOAD CSV FROM 'file:///{}' AS line CALL {{
WITH line

MATCH (from:Method {{class: line[0], name: line[1], sig: coalesce(line[2], ''), source: $source }})

OPTIONAL MATCH (tmpto:Method {{class: line[3], name: line[4], sig: coalesce(line[5], ''), source: $source }})

CALL apoc.do.when(
    tmpto IS NOT NULL,
    "RETURN to AS to",
    "MERGE (new:Method {{class: class, name: name, sig: sig, ret: 'V', source: '{}'}}) ON CREATE SET new.access_flags = 2 RETURN new AS to",
    {{to: tmpto, class: line[3], name: line[4], sig: coalesce(line[5], '')}}
) YIELD value

WITH value.to as to, from

CREATE (from)-[:CALLS {{source: $source}}]->(to)

}} IN TRANSACTIONS OF {} ROWS"#,
            path, FRAMEWORK_SOURCE, TRANSACTION_SIZE
        );

        self.run_binding_stmt(&query, |q| q.param("source", source))?;
        Ok(())
    }
}

impl From<neo4rs::Error> for Error {
    fn from(value: neo4rs::Error) -> Self {
        match value {
            neo4rs::Error::IOError { detail } => Self::Generic(detail.to_string()),
            neo4rs::Error::ConnectionError => Self::ConnectionError,
            neo4rs::Error::UnexpectedMessage(msg) => Self::Generic(msg),
            _ => Self::Generic(value.to_string()),
        }
    }
}

impl Neo4jDatabase {
    /// Create a new Neo4jDatabase instance, connecting to an existing db accessible at `uri`
    /// (including port number) using the provided credentials
    pub(crate) fn connect(uri: &str, user: &str, pass: &str) -> Result<Self> {
        log::trace!("connecting to graph database at {}", uri);
        let rt = Runtime::new().unwrap();
        let graph = rt.block_on(Graph::new(uri, user, pass))?;

        Ok(Neo4jDatabase { rt, graph })
    }

    fn create_constraints_and_indices(&self) -> Result<()> {
        const STATEMENTS: &[&'static str] = &[
            // These are not allowed in community edition. wtf.
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (c:Class) REQUIRE c.name IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (c:Class) REQUIRE c.source IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (c:Class) REQUIRE c.access_flags IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (m:Method) REQUIRE m.source IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (m:Method) REQUIRE m.name IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (m:Method) REQUIRE m.class IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (m:Method) REQUIRE m.sig IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR (m:Method) REQUIRE m.ret IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR ()-[r:HAS_METHOD]-() REQUIRE r.source IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR ()-[r:IMPLEMENTS]-() REQUIRE r.source IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR ()-[r:HAS_CHILD]-() REQUIRE r.source IS NOT NULL",
            //"CREATE CONSTRAINT IF NOT EXISTS FOR ()-[r:CALLS]-() REQUIRE r.source IS NOT NULL",
            concat!(
                "CREATE CONSTRAINT unique_class_name_source IF NOT EXISTS ",
                "FOR (c:Class) REQUIRE (c.name, c.source) IS UNIQUE"
            ),
            concat!(
                "CREATE CONSTRAINT unique_method_signature_source IF NOT EXISTS ",
                "FOR (m:Method) REQUIRE (m.source, m.class, m.name, m.sig, m.ret) IS UNIQUE"
            ),
            "CREATE INDEX class_name IF NOT EXISTS FOR (c:Class) ON c.name",
            "CREATE INDEX method_class_source IF NOT EXISTS FOR (m:Method) ON (m.class, m.source)",
            "CREATE INDEX method_name_source IF NOT EXISTS FOR (m:Method) ON (m.name, m.source)",
            concat!(
                "CREATE INDEX method_name_sig IF NOT EXISTS ",
                "FOR (m:Method) ON (m.name, m.sig)"
            ),
            concat!(
                "CREATE INDEX method_name_sig_source IF NOT EXISTS ",
                "FOR (m:Method) ON (m.name, m.sig, m.source)"
            ),
            concat!(
                "CREATE INDEX method_class_name_sig IF NOT EXISTS ",
                "FOR (m:Method) ON (m.class, m.name, m.sig)"
            ),
            concat!(
                "CREATE INDEX method_class_name_sig_source IF NOT EXISTS ",
                "FOR (m:Method) ON (m.class, m.name, m.sig, m.source)"
            ),
            concat!(
                "CREATE INDEX calls_source IF NOT EXISTS ",
                "FOR ()-[r:CALLS]-() ON (r.source)"
            ),
            concat!(
                "CREATE INDEX implements_source IF NOT EXISTS ",
                "FOR ()-[r:IMPLEMENTS]-() ON (r.source)"
            ),
            concat!(
                "CREATE INDEX has_child_source IF NOT EXISTS ",
                "FOR ()-[r:HAS_CHILD]-() ON (r.source)"
            ),
            concat!(
                "CREATE INDEX has_method_source IF NOT EXISTS ",
                "FOR ()-[r:HAS_METHOD]-() ON (r.source)"
            ),
        ];
        for s in STATEMENTS {
            self.run_stmt(*s)?;
        }
        Ok(())
    }

    fn wipe_database(&self) -> Result<()> {
        log::info!("Wiping graph database");
        log::debug!("Deleting relations...");
        self.run_stmt("CALL { MATCH ()-[r]->() DELETE r } IN TRANSACTIONS OF 1000 ROWS")?;
        log::debug!("Deleting nodes...");
        self.run_stmt("CALL { MATCH (n) DELETE n } IN TRANSACTIONS of 1000 ROWS")?;
        log::info!("graph database wiped");
        Ok(())
    }

    fn run_stmt(&self, stmt: &str) -> Result<()> {
        #[cfg(feature = "trace_db")]
        log::trace!("{}", stmt);
        let query = neo4rs::query(stmt);
        Ok(self.rt.block_on(self.graph.run(query))?)
    }

    fn run_binding_stmt<S>(&self, stmt: &str, setup: S) -> Result<()>
    where
        S: FnOnce(Query) -> Query,
    {
        #[cfg(feature = "trace_db")]
        log::trace!("{}", stmt);
        let query = setup(neo4rs::query(stmt));
        Ok(self.rt.block_on(self.graph.run(query))?)
    }

    /// Executes a cypher query, passing each returned row to the `on_row`
    /// closure.
    ///
    /// The `setup` closure is used to modify the query before it is sent
    fn execute_query<R, S>(&self, query_str: &str, mut on_row: R, setup: S) -> Result<()>
    where
        R: FnMut(Row) -> Result<()>,
        S: FnOnce(Query) -> Query,
    {
        #[cfg(feature = "trace_db")]
        log::trace!("{}", query_str);
        let query = setup(neo4rs::query(query_str));
        let mut row_stream = self.rt.block_on(self.graph.execute(query))?;
        loop {
            match self.rt.block_on(row_stream.next())? {
                None => break,
                Some(r) => on_row(r)?,
            }
        }
        Ok(())
    }

    /// Executes a cypher query, passing each returned row to the `on_row`
    /// closure
    fn execute_simple_query<R>(&self, query_str: &str, on_row: R) -> Result<()>
    where
        R: FnMut(Row) -> Result<()>,
    {
        self.execute_query(query_str, on_row, |q| q)
    }

    fn get_existence_query(&self, csv: LoadCSVKind, src_param: &str, retval: &str) -> String {
        match csv {
            LoadCSVKind::Classes => format!("MATCH (c:Class {{ source: {} }}) RETURN count(c) AS {} LIMIT 1", src_param, retval),
            LoadCSVKind::Methods => format!("MATCH (m:Method {{ source: {} }}) RETURN count(m) AS {} LIMIT 1", src_param, retval),
            LoadCSVKind::Supers => format!("MATCH (:Class)-[r:HAS_CHILD {{ source: {} }}]->(:Class) RETURN count(r) AS {} LIMIT 1", src_param, retval),
            LoadCSVKind::Impls => format!("MATCH (:Class)-[r:IMPLEMENTS {{ source: {} }}]->(:Class) RETURN count(r) AS {} LIMIT 1", src_param, retval),
            LoadCSVKind::Calls => format!("MATCH (:Method)-[r:CALLS {{ source: {} }}]->(:Method) RETURN count(r) AS {} LIMIT 1", src_param, retval),

        }
    }
}

#[cfg(all(test, feature = "neo4j_tests"))]
mod test {
    use super::*;
    use crate::testing::{tmp_context, TestContext};
    use crate::utils::path_must_name;
    use rstest::*;
    use std::fs;
    use std::path::PathBuf;

    #[fixture]
    #[once]
    fn n4j(tmp_context: TestContext) -> Neo4jDatabase {
        let db = Neo4jDatabase::connect("127.0.0.1:7687", "", "").expect("failed to get database");
        db.wipe().expect("failed to wipe db");
        db.initialize().expect("failed to init db");

        // Hard coded, but all of these tests require a running neo4j server
        // anyway so there is a lot to do..
        let td = PathBuf::from("/tmp/dtu_neo4j/import");
        let csv_path = td.join("data.csv");
        let import_path = path_must_name(&csv_path);

        fs::write(&csv_path, FRAMEWORK_CLASSES).expect("failed to write content");
        db.load_classes_csv(&tmp_context, import_path, "framework")
            .expect("failed to load framework classes");
        fs::write(&csv_path, APK_CLASSES).expect("failed to write content");
        db.load_classes_csv(&tmp_context, import_path, "apk")
            .expect("failed to load apk classes");

        fs::write(&csv_path, FRAMEWORK_SUPERS).expect("failed to write content");
        db.load_supers_csv(&tmp_context, import_path, "framework")
            .expect("failed to load framework supers");
        fs::write(&csv_path, APK_SUPERS).expect("failed to write content");
        db.load_supers_csv(&tmp_context, import_path, "apk")
            .expect("failed to load apk supers");

        fs::write(&csv_path, FRAMEWORK_IFACES).expect("failed to write content");
        db.load_impls_csv(&tmp_context, import_path, "framework")
            .expect("failed to load framework impls");
        fs::write(&csv_path, APK_IFACES).expect("failed to write content");
        db.load_impls_csv(&tmp_context, import_path, "apk")
            .expect("failed to load apk impls");

        fs::write(&csv_path, FRAMEWORK_METHODS).expect("failed to write content");
        db.load_methods_csv(&tmp_context, import_path, "framework")
            .expect("failed to load framework methods");
        fs::write(&csv_path, APK_METHODS).expect("failed to write content");
        db.load_methods_csv(&tmp_context, import_path, "apk")
            .expect("failed to load apk methods");

        fs::write(&csv_path, FRAMEWORK_CALLS).expect("failed to write content");
        db.load_calls_csv(&tmp_context, import_path, "framework")
            .expect("failed to load framework calls");
        fs::write(&csv_path, APK_CALLS).expect("failed to write content");
        db.load_calls_csv(&tmp_context, import_path, "apk")
            .expect("failed to load apk calls");

        db
    }

    #[rstest]
    fn test_loaded(n4j: &Neo4jDatabase) {
        assert_eq!(
            n4j.should_load_csv("framework", LoadCSVKind::Classes),
            false
        );

        assert_eq!(n4j.should_load_csv("framework", LoadCSVKind::Supers), false);

        assert_eq!(n4j.should_load_csv("framework", LoadCSVKind::Impls), false);

        assert_eq!(
            n4j.should_load_csv("framework", LoadCSVKind::Methods),
            false
        );

        assert_eq!(n4j.should_load_csv("framework", LoadCSVKind::Calls), false);
    }

    #[rstest]
    fn test_supers_no_source(n4j: &Neo4jDatabase) {
        let parent = ClassName::from("Lparent/A;");
        let mut classes = n4j
            .find_child_classes_of(&parent, None)
            .expect("failed to find children");
        classes.sort_by(|lhs, rhs| {
            lhs.name
                .get_smali_name()
                .partial_cmp(&rhs.name.get_smali_name())
                .unwrap()
        });

        assert_eq!(
            classes
                .iter()
                .map(|it| { it.name.get_java_name().to_string() })
                .collect::<Vec<String>>(),
            vec![
                String::from("class.A"),
                "class.B".into(),
                "class.E".into(),
                "class.F".into(),
                "class.G".into(),
                "class.J".into()
            ]
        );
    }

    #[rstest]
    fn test_supers_source(n4j: &Neo4jDatabase) {
        let parent = ClassName::from("Lparent/A;");
        let mut classes = n4j
            .find_child_classes_of(&parent, Some("framework"))
            .expect("failed to find children");
        classes.sort_by(|lhs, rhs| {
            lhs.name
                .get_smali_name()
                .partial_cmp(&rhs.name.get_smali_name())
                .unwrap()
        });

        assert_eq!(
            classes
                .iter()
                .map(|it| { it.name.get_java_name().to_string() })
                .collect::<Vec<String>>(),
            vec![String::from("class.A"), "class.B".into(), "class.E".into(),]
        );
    }

    #[rstest]
    fn test_impls_no_source(n4j: &Neo4jDatabase) {
        let iface = ClassName::from("Liface/A;");
        let mut classes = n4j
            .find_classes_implementing(&iface, None)
            .expect("failed to find impls");

        classes.sort_by(|lhs, rhs| {
            lhs.name
                .get_smali_name()
                .partial_cmp(&rhs.name.get_smali_name())
                .unwrap()
        });

        assert_eq!(
            classes
                .iter()
                .map(|it| { it.name.get_java_name().to_string() })
                .collect::<Vec<String>>(),
            vec![
                String::from("class.A"),
                "class.E".into(),
                "class.G".into(),
                "class.J".into()
            ]
        );
    }

    #[rstest]
    fn test_impls_source(n4j: &Neo4jDatabase) {
        let iface = ClassName::from("Liface/A;");
        let mut classes = n4j
            .find_classes_implementing(&iface, Some("framework"))
            .expect("failed to find impls");

        classes.sort_by(|lhs, rhs| {
            lhs.name
                .get_smali_name()
                .partial_cmp(&rhs.name.get_smali_name())
                .unwrap()
        });

        assert_eq!(
            classes
                .iter()
                .map(|it| { it.name.get_java_name().to_string() })
                .collect::<Vec<String>>(),
            vec![String::from("class.A"), "class.E".into()]
        );
    }

    const FRAMEWORK_CLASSES: &'static str = r#"Lclass/A;,2
Lclass/B;,2
Lclass/C;,2
Lclass/D;,2
Lclass/E;,2
Liface/A;,32770
Liface/B;,32770
Lparent/A;,2
Lparent/B;,2
LA;,2
LB;,2
LD;,2
LF;,2
LH;,2
LM;,2
LT;,2
"#;

    const APK_CLASSES: &'static str = r#"Lclass/F;,2
Lclass/G;,2
Lclass/H;,2
Lclass/I;,2
Lclass/J;,2
Liface/C;,32770
Liface/D;,32770
Lparent/C;,2
Lparent/D;,2
LZ;,2
LC;,2
LF;,2
LQ;,2"#;

    const FRAMEWORK_SUPERS: &'static str = r#"Lclass/A;,Lparent/A;
Lclass/B;,Lparent/A;
Lclass/C;,Lparent/B;
Lclass/D;,Lclass/C;
Lclass/E;,Lclass/A;"#;

    const APK_SUPERS: &'static str = r#"Lclass/F;,Lparent/A;
Lclass/H;,Lparent/C;
Lclass/I;,Lparent/C;
Lclass/J;,Lparent/D;
Lclass/G;,Lclass/F;
Lclass/J;,Lclass/G;"#;

    const FRAMEWORK_IFACES: &'static str = r#"Lclass/D;,Liface/B;
Lclass/E;,Liface/B;
Lclass/A;,Liface/A;"#;

    const APK_IFACES: &'static str = r#"Lclass/F;,Liface/C;
Lclass/G;,Liface/A;
Lclass/G;,Liface/D;
Lclass/I;,Liface/D;"#;

    const FRAMEWORK_METHODS: &'static str = r#"LA;,A,I
LB;,B,Ljava/lang/String;
LA;,A,I
LB;,C,
LA;,A,I
LB;,H,
LA;,Q,
LB;,B,Ljava/lang/String;
LM;,A,I
LB;,B,Ljava/lang/String;
LB;,B,Ljava/lang/String;
LD;,D,
LB;,B,Ljava/lang/String;
LA;,A,I
LB;,B,
LF;,F,
LH;,H,
LB;,B,
LD;,D,
LT;,T,"#;

    const APK_METHODS: &'static str = r#"LZ;,Z,
LC;,C,
LC;,C,
LC;,C,
LF;,F,
LQ;,Q,"#;

    const FRAMEWORK_CALLS: &'static str = r#"LA;,A,I,LB;,B,Ljava/lang/String;
LA;,A,I,LB;,C,
LA;,A,I,LB;,H,
LA;,Q,,LB;,B,Ljava/lang/String;
LM;,A,I,LB;,B,Ljava/lang/String;
LB;,B,Ljava/lang/String;,LD;,D,
LB;,B,Ljava/lang/String;,LA;,A,I
LB;,B,,LF;,F,
LH;,H,,LB;,B,
LD;,D,,LT;,T,"#;

    const APK_CALLS: &'static str = r#"LZ;,Z,,LP;,P,
LC;,C,,LE;,E,
LC;,C,,LF;,F,
LC;,C,,LQ;,Q,
LF;,F,,LT;,T,
LQ;,Q,,LT;,T,"#;
}
