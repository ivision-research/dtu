use std::collections::{HashMap, HashSet};
use std::fs;

use diesel::backend::Backend;
use diesel::dsl::{AsSelect, InnerJoin, InnerJoinOn, IntoBoxed, Select};
use diesel::sql_query;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::sqlite::Sqlite;
use diesel::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations};
use itertools::Itertools;
use smalisa::AccessFlag;

use super::schema::*;
use crate::db::common::Db;
use crate::db::common::*;
use crate::db::graph::models::{
    ClassId, ClassSearch, FieldId, FieldRef, FieldSearchParams, MethodCallPath, MethodId,
    MethodSearch, MethodSearchParams, MethodSpec, SourceId, SourcedString,
};
use crate::db::graph::models::{FieldAccessOp, FieldSearch, FieldSpec, Source};
use crate::db::graph::{ClassSpec, GraphDatabase, StringSearch};
use crate::db::macros::{impl_delete_by, impl_get_all, query};
use crate::utils::{path_must_name, path_must_str, ClassName};
use crate::Context;
use diesel::prelude::*;

pub static GRAPH_DATABASE_FILE_NAME: &'static str = "graph.db";
const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/graph_migrations/");

#[cfg(test)]
const TEST_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/test_graph_migrations/");

pub struct GraphSqliteDatabase {
    db: Db,
}

impl GraphSqliteDatabase {
    pub fn new(ctx: &dyn Context) -> Result<Self> {
        let db = Self {
            db: Db::new(
                ctx,
                GRAPH_DATABASE_FILE_NAME,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        };
        Ok(db)
    }

    pub fn new_from_path<S: AsRef<str> + ?Sized>(path: &S) -> Result<Self> {
        let db = Self {
            db: Db::new_from_path(
                path,
                MIGRATIONS,
                #[cfg(test)]
                TEST_MIGRATIONS,
            )?,
        };
        Ok(db)
    }

    #[cfg(test)]
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

    /// Read using any available connection
    #[inline]
    pub fn query<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut SqliteConnection) -> Result<R>,
    {
        self.db.query(f)
    }

    /// Write, holding one connection for the whole closure inside a transaction
    #[inline]
    pub fn write<F, R, E>(&self, f: F) -> std::result::Result<R, E>
    where
        F: FnOnce(&mut SqliteConnection) -> std::result::Result<R, E>,
        E: From<Error> + From<diesel::result::Error>,
    {
        self.db.write(f)
    }

    /// [GraphSqliteDatabase::write], with `pragmas` applied before the transaction opens
    #[inline]
    pub(super) fn write_with_pragmas<F, R, E>(
        &self,
        pragmas: &str,
        f: F,
    ) -> std::result::Result<R, E>
    where
        F: FnOnce(&mut SqliteConnection) -> std::result::Result<R, E>,
        E: From<Error> + From<diesel::result::Error>,
    {
        self.db.write_with_pragmas(pragmas, f)
    }

    #[allow(unused)]
    pub(super) fn get_source_id(&self, source: &str) -> Result<SourceId> {
        self.query(|c| {
            Ok(query!(sources::table
                .filter(sources::name.eq(source))
                .select(sources::id)
                .limit(1))
            .get_result::<SourceId>(c)?)
        })
    }

    impl_get_all!(get_sources, Source, sources);
    impl_delete_by!(delete_source_by_name, &str, sources, name.eq);

    fn get_class_ids_sql(search: &ClassSearch) -> &'static str {
        match search.source {
            Some(_) => "SELECT c.id FROM classes AS c JOIN sources AS s ON c.source = s.id WHERE c.name = ? AND s.name = ?",
            None => "SELECT id FROM classes WHERE name = ?",
        }
    }

    fn get_method_for_string_like(&self, string: &str) -> Result<Vec<MethodSpec>> {
        let q = query!(strings::table
            .filter(strings::string.like(string))
            .inner_join(method_strings::table)
            .inner_join(methods::table.on(method_strings::method.eq(methods::id)))
            .inner_join(classes::table.on(classes::id.eq(methods::class)))
            .inner_join(sources::table)
            .select(MethodSpecRow::as_select()));
        let rows = self.query(|c| Ok(q.load::<MethodSpecRow>(c)?))?;
        Ok(rows.into_iter().map(MethodSpec::from).collect())
    }

    fn get_method_for_string_eq(&self, string: &str) -> Result<Vec<MethodSpec>> {
        let q = query!(strings::table
            .filter(strings::string.eq(string))
            .inner_join(method_strings::table)
            .inner_join(methods::table.on(method_strings::method.eq(methods::id)))
            .inner_join(classes::table.on(classes::id.eq(methods::class)))
            .inner_join(sources::table)
            .select(MethodSpecRow::as_select()));
        let rows = self.query(|c| Ok(q.load::<MethodSpecRow>(c)?))?;
        Ok(rows.into_iter().map(MethodSpec::from).collect())
    }

    fn find_strings_like(&self, string: &str, source: Option<&str>) -> Result<Vec<SourcedString>> {
        match source {
            None => {
                let q = query!(strings::table
                    .filter(strings::string.like(string))
                    .inner_join(sources::table)
                    .select(SourcedString::as_select()));
                Ok(self
                    .query(|c| Ok(q.load(c)?))?
                    .into_iter()
                    .collect::<Vec<_>>())
            }
            Some(v) => {
                let q = query!(strings::table
                    .filter(strings::string.like(string))
                    .inner_join(sources::table)
                    .filter(sources::name.eq(v))
                    .select(SourcedString::as_select()));
                Ok(self
                    .query(|c| Ok(q.load(c)?))?
                    .into_iter()
                    .collect::<Vec<_>>())
            }
        }
    }

    fn find_strings_eq(&self, string: &str, source: Option<&str>) -> Result<Vec<SourcedString>> {
        match source {
            None => {
                let q = query!(strings::table
                    .filter(strings::string.eq(string))
                    .inner_join(sources::table)
                    .select(SourcedString::as_select()));
                Ok(self
                    .query(|c| Ok(q.load(c)?))?
                    .into_iter()
                    .collect::<Vec<_>>())
            }
            Some(v) => {
                let q = query!(strings::table
                    .filter(strings::string.eq(string))
                    .inner_join(sources::table)
                    .filter(sources::name.eq(v))
                    .select(SourcedString::as_select()));
                Ok(self
                    .query(|c| Ok(q.load(c)?))?
                    .into_iter()
                    .collect::<Vec<_>>())
            }
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
        let int_depth: i32 = depth
            .try_into()
            .map_err(|_| Error::Generic(format!("invalid depth")))?;

        let mid_query = method.id_query();
        let method_ids = self.query(|c| Ok(mid_query.load::<MethodId>(c)?))?;

        if method_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Note that UNION is required over UNION ALL since the call graph can be cyclic.
        let q = query!(sql_query(format!(
            r#"WITH RECURSIVE
    search_methods(search_method_id) AS (SELECT value FROM json_each(?1)),
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
        WHERE ct.distance < ?2
        ORDER BY 2 DESC
    )
SELECT ct.path FROM calls_to AS ct WHERE ct.distance > 0;"#,
        ))
        .bind::<Text, _>(to_json_array(&method_ids))
        .bind::<Integer, _>(int_depth));

        let rows = self.query(|c| Ok(q.get_results::<RouteRow>(c)?))?;
        let mut routes = self.hydrate_routes(rows)?;

        // A call-into route is built outwards from the searched method, so it reads backwards
        if matches!(dir, CallDirection::Into) {
            for route in routes.iter_mut() {
                route.reverse();
            }
        }

        Ok(routes
            .into_iter()
            .filter(|route| match call_source {
                None => true,
                Some(src) => route.first().is_some_and(|it| it.source == src),
            })
            .map(MethodCallPath::from)
            .collect())
    }

    /// Turn routes given as JSON arrays of method ids into their full specs
    ///
    /// A method usually appears in many routes, so each distinct id is fetched once and cloned
    /// into place rather than hydrated once per occurrence.
    fn hydrate_routes(&self, rows: Vec<RouteRow>) -> Result<Vec<Vec<MethodSpec>>> {
        let routes = rows
            .into_iter()
            .map(|it| it.method_ids())
            .collect::<Result<Vec<Vec<MethodId>>>>()?;

        // Every method present in the results, so they can be fetched in one go. Bounded by the
        // distinct methods the routes touch, which is always small relative to the route count,
        // so `eq_any` can't approach the bind parameter limit.
        let unique = routes
            .iter()
            .flatten()
            .copied()
            .unique()
            .collect::<Vec<MethodId>>();

        let specs = self
            .get_methods_by_id(&unique)?
            .into_iter()
            .map(|it| (it.id, it))
            .collect::<HashMap<MethodId, MethodSpec>>();

        Ok(routes
            .into_iter()
            .map(|route| {
                route
                    .into_iter()
                    .filter_map(|id| specs.get(&id).cloned())
                    .collect()
            })
            .collect())
    }
}

enum CallDirection {
    From,
    Into,
}

impl<'a> FieldSearchParams<'a> {
    fn class(&self) -> Option<&'a ClassName> {
        match self {
            Self::ByClass { class }
            | Self::ByClassAndName { class, .. }
            | Self::ByFullSpec { class, .. } => Some(*class),
        }
    }

    fn name(&self) -> Option<&'a str> {
        match self {
            Self::ByClassAndName { name, .. } | Self::ByFullSpec { name, .. } => Some(*name),
            Self::ByClass { .. } => None,
        }
    }

    fn type_(&self) -> Option<&'a str> {
        match self {
            Self::ByFullSpec { ty, .. } => Some(*ty),
            Self::ByClassAndName { .. } | Self::ByClass { .. } => None,
        }
    }
}

type FieldQuerySource = InnerJoinOn<
    InnerJoin<class_fields::table, classes::table>,
    sources::table,
    diesel::dsl::Eq<sources::id, classes::source>,
>;

type BoxedFieldQuery<'a, S> = IntoBoxed<'a, Select<FieldQuerySource, S>, Sqlite>;

impl<'a> FieldSearch<'a> {
    /// A query for the ids of every method this search matches
    fn id_query(&self) -> BoxedFieldQuery<'a, class_fields::id> {
        let mut q = class_fields::table
            .inner_join(classes::table)
            .inner_join(sources::table.on(sources::id.eq(classes::source)))
            .select(class_fields::id)
            .into_boxed();

        if let Some(class) = self.param.class() {
            q = q.filter(classes::name.eq(class.get_smali_name()));
        }

        if let Some(name) = self.param.name() {
            q = q.filter(class_fields::name.eq(name));
        }

        if let Some(ty) = self.param.type_() {
            q = q.filter(class_fields::ty.eq(ty));
        }
        if let Some(source) = self.source {
            q = q.filter(sources::name.eq(source));
        }
        query!(q)
    }

    /// A query for the full spec of every field this search matches
    fn spec_query(&self) -> BoxedFieldQuery<'a, AsSelect<FieldSpecRow, Sqlite>> {
        let mut q = class_fields::table
            .inner_join(classes::table)
            .inner_join(sources::table.on(sources::id.eq(classes::source)))
            .select(FieldSpecRow::as_select())
            .into_boxed();

        if let Some(class) = self.param.class() {
            q = q.filter(classes::name.eq(class.get_smali_name()));
        }

        if let Some(name) = self.param.name() {
            q = q.filter(class_fields::name.eq(name));
        }

        if let Some(ty) = self.param.type_() {
            q = q.filter(class_fields::ty.eq(ty));
        }

        if let Some(source) = self.source {
            q = q.filter(sources::name.eq(source));
        }

        query!(q)
    }
}

impl<'a> MethodSearchParams<'a> {
    fn class(&self) -> Option<&'a ClassName> {
        match self {
            Self::ByClass { class }
            | Self::ByClassAndName { class, .. }
            | Self::ByFullSpec { class, .. } => Some(*class),
            Self::ByName { .. } | Self::ByNameAndSignature { .. } => None,
        }
    }

    fn name(&self) -> Option<&'a str> {
        match self {
            Self::ByName { name }
            | Self::ByClassAndName { name, .. }
            | Self::ByNameAndSignature { name, .. }
            | Self::ByFullSpec { name, .. } => Some(*name),
            Self::ByClass { .. } => None,
        }
    }

    fn signature(&self) -> Option<&'a str> {
        match self {
            Self::ByNameAndSignature { signature, .. } | Self::ByFullSpec { signature, .. } => {
                Some(*signature)
            }
            Self::ByName { .. } | Self::ByClass { .. } | Self::ByClassAndName { .. } => None,
        }
    }
}

type MethodQuerySource = InnerJoin<InnerJoin<methods::table, classes::table>, sources::table>;

type BoxedMethodQuery<'a, S> = IntoBoxed<'a, Select<MethodQuerySource, S>, Sqlite>;

impl<'a> MethodSearch<'a> {
    /// A query for the ids of every method this search matches
    fn id_query(&self) -> BoxedMethodQuery<'a, methods::id> {
        let mut q = methods::table
            .inner_join(classes::table)
            .inner_join(sources::table)
            .select(methods::id)
            .into_boxed();

        if let Some(class) = self.param.class() {
            q = q.filter(classes::name.eq(class.get_smali_name()));
        }
        if let Some(name) = self.param.name() {
            q = q.filter(methods::name.eq(name));
        }
        if let Some(signature) = self.param.signature() {
            q = q.filter(methods::args.eq(signature));
        }
        if let Some(ret) = self.ret {
            q = q.filter(methods::ret.eq(ret));
        }
        if let Some(source) = self.source {
            q = q.filter(sources::name.eq(source));
        }
        query!(q)
    }

    /// A query for the full spec of every method this search matches
    fn spec_query(&self) -> BoxedMethodQuery<'a, AsSelect<MethodSpecRow, Sqlite>> {
        let mut q = methods::table
            .inner_join(classes::table)
            .inner_join(sources::table)
            .select(MethodSpecRow::as_select())
            .into_boxed();

        if let Some(class) = self.param.class() {
            q = q.filter(classes::name.eq(class.get_smali_name()));
        }
        if let Some(name) = self.param.name() {
            q = q.filter(methods::name.eq(name));
        }
        if let Some(signature) = self.param.signature() {
            q = q.filter(methods::args.eq(signature));
        }
        if let Some(ret) = self.ret {
            q = q.filter(methods::ret.eq(ret));
        }
        if let Some(source) = self.source {
            q = q.filter(sources::name.eq(source));
        }
        query!(q)
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

    fn find_callers_from(
        &self,
        method: &MethodSearch,
        methods: &[MethodId],
    ) -> Result<Vec<MethodCallPath>> {
        if methods.is_empty() {
            return Ok(Vec::new());
        }

        let mid_query = method.id_query();
        let method_ids = self.query(|c| Ok(mid_query.load::<MethodId>(c)?))?;

        if method_ids.is_empty() {
            return Ok(Vec::new());
        }

        let entry_json = to_json_array(methods);
        let targets_json = to_json_array(&method_ids);

        // The first part of this query answers a reachability question and serves to as an initial
        // filter. This prevents constructing paths for dead ends and allows us to do this without a
        // depth.
        //
        //      entry_reachable_methods - Every method that can be reached from all of the
        //                                entrypoint methods
        //
        //      direct_target_callers - All methods that contain a call to a target method and are also
        //                              reachable from an entry method. Note that the weird `IN
        //                              (SELECT ...) is actually important to the performance of
        //                              this query! Without it the query takes significantly longer,
        //                              because of the JSON load.
        //
        //      dtc_reaching_methods - All methods that can reach the direct target callers
        //
        //      relevant_methods - All methods that are reachable from an entrypoint and also
        //                         reach a direct target caller

        let q = sql_query(
            r#"WITH RECURSIVE

    entry(id) AS (SELECT value FROM json_each(?1)),

    target_methods(id) AS (SELECT value FROM json_each(?2)),

    entry_reachable_methods(id) AS (
        SELECT id FROM entry
        UNION
        SELECT c.callee
        FROM calls AS c
        JOIN entry_reachable_methods AS r 
            ON c.caller = r.id
    ),

    direct_target_callers(id) AS (
        SELECT DISTINCT c.caller
        FROM calls AS c
        JOIN entry_reachable_methods AS r
            ON r.id = c.caller
        WHERE c.callee IN (SELECT id FROM target_methods)
    ),

    dtc_reaching_methods(id) AS (
        SELECT id FROM direct_target_callers
        UNION
        SELECT c.caller
        FROM calls AS c
        JOIN dtc_reaching_methods AS t
            ON c.callee = t.id
    ),

    relevant_methods(id) AS (
        SELECT id FROM entry_reachable_methods
        INTERSECT
        SELECT id FROM dtc_reaching_methods
    ),

    route(id, path, depth) AS (
        SELECT e.id, json_array(e.id), 0
        FROM entry AS e
        JOIN relevant_methods AS rel
            ON rel.id = e.id
        UNION ALL
        SELECT
            c.callee,
            json_insert(r.path, '$[#]', c.callee),
            r.depth + 1
        FROM route AS r
        JOIN calls AS c
            ON r.id = c.caller
        JOIN relevant_methods AS rel
            ON rel.id = c.callee
        WHERE
            NOT EXISTS (SELECT 1 FROM json_each(r.path) je WHERE je.value = c.callee)
    )

SELECT r.path
FROM route AS r
JOIN direct_target_callers AS d ON d.id = r.id
ORDER BY r.id, r.depth;
    "#,
        )
        .bind::<Text, _>(entry_json)
        .bind::<Text, _>(targets_json);

        let q = query!(q);

        let rows = self.query(|c| Ok(q.get_results::<RouteRow>(c)?))?;

        Ok(self
            .hydrate_routes(rows)?
            .into_iter()
            .map(MethodCallPath::from)
            .collect())
    }

    fn find_field_refs_from(
        &self,
        field: &FieldSearch,
        action: FieldAccessOp,
        methods: &[MethodId],
    ) -> Result<Vec<MethodCallPath>> {
        // This implementation is very similar to the method one, the only difference is
        // `direct_field_users`
        if methods.is_empty() {
            return Ok(Vec::new());
        }

        let fid_query = field.id_query();
        let field_ids = self.query(|c| Ok(fid_query.load::<FieldId>(c)?))?;

        if field_ids.is_empty() {
            return Ok(Vec::new());
        }

        let entry_json = to_json_array(methods);
        let targets_json = to_json_array(&field_ids);

        let q = sql_query(
            r#"WITH RECURSIVE

    entry(id) AS (SELECT value FROM json_each(?1)),

    target_fields(id) AS (SELECT value FROM json_each(?2)),

    entry_reachable_methods(id) AS (
        SELECT id FROM entry
        UNION
        SELECT c.callee
        FROM calls AS c
        JOIN entry_reachable_methods AS r 
            ON c.caller = r.id
    ),

    direct_field_users(id) AS (
      SELECT DISTINCT mfa.method
      FROM method_field_access AS mfa
      JOIN entry_reachable_methods AS er ON er.id = mfa.method
      WHERE mfa.field IN (SELECT id FROM target_fields) AND mfa.action = ?3
    ),

    dfu_reaching_methods(id) AS (
        SELECT id FROM direct_field_users
        UNION
        SELECT c.caller
        FROM calls AS c
        JOIN dfu_reaching_methods AS t
            ON c.callee = t.id
    ),

    relevant_methods(id) AS (
        SELECT id FROM entry_reachable_methods
        INTERSECT
        SELECT id FROM dfu_reaching_methods
    ),

    route(id, path, depth) AS (
        SELECT e.id, json_array(e.id), 0
        FROM entry AS e
        JOIN relevant_methods AS rel
            ON rel.id = e.id
        UNION ALL
        SELECT
            c.callee,
            json_insert(r.path, '$[#]', c.callee),
            r.depth + 1
        FROM route AS r
        JOIN calls AS c
            ON r.id = c.caller
        JOIN relevant_methods AS rel
            ON rel.id = c.callee
        WHERE
            NOT EXISTS (SELECT 1 FROM json_each(r.path) je WHERE je.value = c.callee)
    )

SELECT r.path
FROM route AS r
JOIN direct_field_users AS d ON d.id = r.id
ORDER BY r.id, r.depth;
    "#,
        )
        .bind::<Text, _>(entry_json)
        .bind::<Text, _>(targets_json)
        .bind::<Integer, _>(action as i32);

        let q = query!(q);

        let rows = self.query(|c| Ok(q.get_results::<RouteRow>(c)?))?;

        Ok(self
            .hydrate_routes(rows)?
            .into_iter()
            .map(MethodCallPath::from)
            .collect())
    }

    fn wipe(&self, ctx: &dyn Context) -> Result<()> {
        let path = ctx.get_sqlite_dir()?;
        for elem in walkdir::WalkDir::new(path) {
            let Ok(elem) = elem else { continue };
            let path = elem.path();
            let fname = path_must_name(path);
            if fname.starts_with(GRAPH_DATABASE_FILE_NAME) {
                log::info!("removing {}", path_must_str(path));
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
    fn remove_source(&self, source: &str) -> Result<()> {
        Ok(self.delete_source_by_name(source)?)
    }

    fn get_class_by_id(&self, id: ClassId) -> Result<ClassSpec> {
        let q = query!(classes::table
            .filter(classes::id.eq(id))
            .inner_join(sources::table)
            .select(ClassSpecRow::as_select()));

        Ok(self
            .query(|c| Ok(q.first::<ClassSpecRow>(c)?))
            .map(ClassSpec::from)?)
    }

    fn get_classes_by_id(&self, ids: &[ClassId]) -> Result<Vec<ClassSpec>> {
        let q = query!(classes::table
            .filter(classes::id.eq_any(ids))
            .inner_join(sources::table)
            .select(ClassSpecRow::as_select()));
        Ok(self
            .query(|c| Ok(q.load::<ClassSpecRow>(c)?))?
            .into_iter()
            .map(ClassSpec::from)
            .collect::<Vec<_>>())
    }

    fn get_method_by_id(&self, id: MethodId) -> Result<MethodSpec> {
        let q = query!(methods::table
            .filter(methods::id.eq(id))
            .inner_join(classes::table)
            .inner_join(sources::table)
            .select(MethodSpecRow::as_select()));

        Ok(self
            .query(|c| Ok(q.first::<MethodSpecRow>(c)?))
            .map(MethodSpec::from)?)
    }

    fn get_methods_by_id(&self, ids: &[MethodId]) -> Result<Vec<MethodSpec>> {
        let q = query!(methods::table
            .filter(methods::id.eq_any(ids))
            .inner_join(classes::table)
            .inner_join(sources::table)
            .select(MethodSpecRow::as_select()));
        Ok(self
            .query(|c| Ok(q.load::<MethodSpecRow>(c)?))?
            .into_iter()
            .map(MethodSpec::from)
            .collect::<Vec<_>>())
    }

    fn get_method_ids(&self, search: &MethodSearch) -> Result<Vec<MethodId>> {
        let query = search.id_query();
        Ok(self.query(|c| Ok(query.load::<MethodId>(c)?))?)
    }

    fn get_field_ids(&self, search: &FieldSearch) -> Result<Vec<FieldId>> {
        let query = search.id_query();
        Ok(self.query(|c| Ok(query.load::<FieldId>(c)?))?)
    }

    fn get_fields(&self, search: &FieldSearch) -> Result<Vec<FieldSpec>> {
        let query = search.spec_query();
        let rows: Vec<FieldSpecRow> = self.query(|c| Ok(query.load::<FieldSpecRow>(c)?))?;
        Ok(rows.into_iter().map(FieldSpec::from).collect())
    }

    fn get_methods(&self, search: &MethodSearch) -> Result<Vec<MethodSpec>> {
        let query = search.spec_query();
        let rows: Vec<MethodSpecRow> = self.query(|c| Ok(query.load::<MethodSpecRow>(c)?))?;
        Ok(rows.into_iter().map(MethodSpec::from).collect())
    }

    fn get_method_source_or_framework(
        &self,
        class: &ClassName,
        name: &str,
        args: &str,
        return_type: &str,
        source: &str,
    ) -> Result<Option<MethodSpec>> {
        let is_framework = source == FRAMEWORK_SOURCE;

        if is_framework {
            let q = query!(methods::table
                .inner_join(sources::table)
                .inner_join(classes::table)
                .select(MethodSpecRow::as_select())
                .filter(methods::name.eq(name))
                .filter(methods::args.eq(args))
                .filter(methods::ret.eq(return_type))
                .filter(classes::name.eq(class.get_smali_name()))
                .filter(sources::name.eq(source)));
            return self.query(|c| {
                Ok(q.first::<MethodSpecRow>(c)
                    .optional()?
                    .map(MethodSpec::from))
            });
        }

        let q = query!(methods::table
            .inner_join(sources::table)
            .inner_join(classes::table)
            .select(MethodSpecRow::as_select())
            .filter(methods::name.eq(name))
            .filter(methods::args.eq(args))
            .filter(methods::ret.eq(return_type))
            .filter(classes::name.eq(class.get_smali_name()))
            .filter(sources::name.eq_any([source, FRAMEWORK_SOURCE]))
            .order((sources::name.eq(source).desc(), methods::id.asc())));

        self.query(|c| {
            Ok(q.first::<MethodSpecRow>(c)
                .optional()?
                .map(MethodSpec::from))
        })
    }

    fn get_method_field_refs(&self, method: MethodId) -> Result<Vec<FieldRef>> {
        let q = query!(method_field_access::table
            .filter(method_field_access::method.eq(method))
            .inner_join(class_fields::table.on(class_fields::id.eq(method_field_access::field)))
            .inner_join(classes::table.on(classes::id.eq(class_fields::class)))
            .inner_join(sources::table.on(sources::id.eq(classes::source)))
            .select((FieldSpecRow::as_select(), method_field_access::action)));

        Ok(self
            .query(|c| Ok(q.load::<(FieldSpecRow, i32)>(c)?))?
            .into_iter()
            .filter_map(|it| {
                let op = FieldAccessOp::maybe_from_literal(it.1 as u8)?;
                Some(FieldRef {
                    field: it.0.into(),
                    op,
                })
            })
            .collect())
    }

    fn get_methods_referencing_field(
        &self,
        field: FieldId,
        action: Option<FieldAccessOp>,
    ) -> Result<Vec<MethodSpec>> {
        let mut q = class_fields::table
            .filter(class_fields::id.eq(field))
            .inner_join(
                method_field_access::table.on(method_field_access::field.eq(class_fields::id)),
            )
            .inner_join(methods::table.on(methods::id.eq(method_field_access::method)))
            .inner_join(classes::table.on(classes::id.eq(methods::class)))
            .inner_join(sources::table.on(sources::id.eq(classes::source)))
            .select(MethodSpecRow::as_select())
            .into_boxed();

        if let Some(v) = action {
            q = q.filter(method_field_access::action.eq(v as i32));
        }

        let q = query!(q);

        let rows = self.query(|c| Ok(q.load::<MethodSpecRow>(c)?))?;
        Ok(rows.into_iter().map(MethodSpec::from).collect())
    }

    fn get_strings_for_method(&self, method: MethodId) -> Result<Vec<String>> {
        let q = query!(method_strings::table
            .inner_join(strings::table)
            .filter(method_strings::method.eq(method))
            .select(strings::string));

        Ok(self.query(|c| Ok(q.load::<String>(c)?))?)
    }

    fn get_strings_for_source(&self, source: &str) -> Result<Vec<String>> {
        let q = query!(strings::table
            .inner_join(sources::table)
            .filter(sources::name.eq(source))
            .select(strings::string));
        Ok(self.query(|c| Ok(q.load::<String>(c)?))?)
    }

    fn find_strings(
        &self,
        string: StringSearch,
        source: Option<&str>,
    ) -> Result<Vec<SourcedString>> {
        match string {
            StringSearch::Exact(s) => self.find_strings_eq(s, source),
            StringSearch::Like(s) => self.find_strings_like(s, source),
        }
    }

    fn get_methods_for_string(&self, string: StringSearch) -> Result<Vec<MethodSpec>> {
        match string {
            StringSearch::Exact(s) => self.get_method_for_string_eq(s),
            StringSearch::Like(s) => self.get_method_for_string_like(s),
        }
    }

    fn get_all_sources(&self) -> Result<HashSet<String>> {
        let sources = self.get_sources()?;
        let mut m = HashSet::with_capacity(sources.len());
        m.extend(sources.into_iter().map(|it| it.name));
        Ok(m)
    }

    fn get_classes_for(&self, source: &str) -> Result<Vec<ClassName>> {
        let q = query!(classes::table
            .inner_join(sources::table)
            .filter(sources::name.eq(source))
            .select(classes::name));
        Ok(self.query(|c| Ok(q.load::<ClassName>(c)?))?)
    }

    fn find_classes_with_method(
        &self,
        name: &str,
        args: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>> {
        let mut q = classes::table
            .inner_join(sources::table.on(classes::source.eq(sources::id)))
            .inner_join(methods::table.on(methods::class.eq(classes::id)))
            .select(ChildClassRow::as_select())
            .filter(methods::name.eq(name))
            .into_boxed();

        if let Some(v) = args {
            q = q.filter(methods::args.eq(v));
        }

        if let Some(s) = source {
            q = q.filter(sources::name.eq(s));
        }
        let q = query!(q);
        let rows: Vec<ChildClassRow> = self.query(|c| Ok(q.get_results(c)?))?;
        Ok(rows.into_iter().map(ClassSpec::from).collect())
    }

    fn get_methods_for(&self, source: &str) -> Result<Vec<MethodSpec>> {
        let q = query!(methods::table
            .inner_join(sources::table)
            .inner_join(classes::table)
            .filter(sources::name.eq(source))
            .select(MethodSpecRow::as_select()));
        let rows = self.query(|c| Ok(q.load::<MethodSpecRow>(c)?))?;
        Ok(rows.into_iter().map(MethodSpec::from).collect())
    }

    fn find_outgoing_calls(
        &self,
        from: &MethodSearch,
        depth: usize,
    ) -> Result<Vec<MethodCallPath>> {
        self.get_calls(CallDirection::From, from, None, depth)
    }

    fn find_interfaces_of(&self, class: &ClassSearch) -> Result<Vec<ClassSpec>> {
        let get_class_ids_sql = Self::get_class_ids_sql(class);

        // UNION rather than UNION ALL: an interface reachable by more than one
        // route through the hierarchy would otherwise keep re-expanding
        let mut q = sql_query(format!(
            r#"WITH RECURSIVE
    search_classes(search_class_id) AS ({get_class_ids_sql}),

    ancestors(classid) AS (
        SELECT search_class_id FROM search_classes
        UNION
        SELECT s.parent
        FROM supers AS s
        JOIN ancestors AS a
            ON a.classid = s.child
        UNION
        SELECT i.interface
        FROM interfaces AS i
        JOIN ancestors AS a
            ON a.classid = i.class
    ),

    class_specs(source, name, access_flags) AS (
        SELECT s.name, c.name, c.access_flags
        FROM interfaces AS i
        JOIN ancestors AS a
            ON a.classid = i.class
        JOIN classes AS c
            ON c.id = i.interface
        JOIN sources AS s
            ON s.id = c.source
    )

SELECT DISTINCT source, name, access_flags from class_specs
    "#
        ))
        .into_boxed();

        q = q.bind::<Text, _>(class.class.get_smali_name());
        if let Some(src) = class.source {
            q = q.bind::<Text, _>(src);
        }

        self.query(|c| {
            let rows: Vec<ChildClassRow> = query!(q).get_results(c)?;
            Ok(rows.into_iter().map(ClassSpec::from).collect())
        })
    }

    fn find_parent_classes_of(&self, child: &ClassName, source: &str) -> Result<Vec<ClassSpec>> {
        // Same note as the child search with the UNION ALL, there shouldn't be cycles in well
        // formed data
        let q = sql_query(
            r#"WITH RECURSIVE

    child_class(child_id) AS (
        SELECT c.id
        FROM classes AS c
        JOIN sources AS s
            ON c.source = s.id
        WHERE c.name = ?1 AND s.name = ?2
    ),

    parents(classid, distance) AS (
        SELECT child_id, 0 FROM child_class
        UNION ALL
        SELECT
            s.parent,
            p.distance + 1
        FROM supers AS s
        JOIN parents AS p
            ON p.classid = s.child
    ),

    class_specs(source, name, access_flags) AS (
        SELECT s.name, c.name, c.access_flags
        FROM parents AS p
        JOIN classes AS c
            ON c.id = p.classid
        JOIN sources AS s
            on s.id = c.source
        WHERE p.distance > 0
    )

SELECT DISTINCT source, name, access_flags from class_specs
    "#,
        )
        .bind::<Text, _>(child.get_smali_name())
        .bind::<Text, _>(source);

        let q = query!(q);

        let rows: Vec<ChildClassRow> = self.query(|c| Ok(q.get_results(c)?))?;
        Ok(rows.into_iter().map(ClassSpec::from).collect())
    }

    fn find_child_classes_of(
        &self,
        parent: &ClassSearch,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>> {
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

        let q = query!(q);
        let rows: Vec<ChildClassRow> = self.query(|c| Ok(q.get_results(c)?))?;

        Ok(rows.into_iter().map(ClassSpec::from).collect())
    }

    fn find_classes_implementing(
        &self,
        iface: &ClassSearch,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>> {
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

        let q = query!(q);
        let rows: Vec<ChildClassRow> = self.query(|c| Ok(q.get_results(c)?))?;
        Ok(rows.into_iter().map(ClassSpec::from).collect())
    }
}

impl<DB: Backend> Selectable<DB> for SourcedString {
    type SelectExpression = (strings::string, sources::name);

    fn construct_selection() -> Self::SelectExpression {
        (strings::string, sources::name)
    }
}

#[derive(Queryable, Debug)]
struct ClassSpecRow {
    #[diesel(sql_type = Text)]
    name: ClassName,
    #[diesel(sql_type = BigInt)]
    access_flags: i64,
    #[diesel(sql_type = Text)]
    source: String,
}

impl<DB: Backend> Selectable<DB> for ClassSpecRow {
    type SelectExpression = (classes::name, classes::access_flags, sources::name);

    fn construct_selection() -> Self::SelectExpression {
        (classes::name, classes::access_flags, sources::name)
    }
}

impl From<ClassSpecRow> for ClassSpec {
    fn from(value: ClassSpecRow) -> Self {
        ClassSpec {
            name: value.name,
            access_flags: AccessFlag::from_bits_truncate(value.access_flags as u64),
            source: value.source,
        }
    }
}

#[derive(Queryable, Debug)]
struct FieldSpecRow {
    #[diesel(sql_type = Integer)]
    id: FieldId,
    #[diesel(sql_type = Text)]
    class: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    ty: String,
    #[diesel(sql_type = BigInt)]
    access_flags: i64,
    #[diesel(sql_type = Text)]
    source: String,
}

impl<DB: Backend> Selectable<DB> for FieldSpecRow {
    type SelectExpression = (
        class_fields::id,
        classes::name,
        class_fields::name,
        class_fields::ty,
        class_fields::access_flags,
        sources::name,
    );

    fn construct_selection() -> Self::SelectExpression {
        (
            class_fields::id,
            classes::name,
            class_fields::name,
            class_fields::ty,
            class_fields::access_flags,
            sources::name,
        )
    }
}

impl From<FieldSpecRow> for FieldSpec {
    fn from(value: FieldSpecRow) -> Self {
        FieldSpec {
            class: value.class.into(),
            name: value.name,
            id: value.id,
            ty: value.ty,
            access_flags: AccessFlag::from_bits_truncate(value.access_flags as u64),
            source: value.source,
        }
    }
}

#[derive(Queryable, Debug)]
struct MethodSpecRow {
    #[diesel(sql_type = Integer)]
    class_id: ClassId,
    #[diesel(sql_type = Text)]
    class: String,
    #[diesel(sql_type = Integer)]
    id: MethodId,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    args: String,
    #[diesel(sql_type = Text)]
    ret: String,
    #[diesel(sql_type = BigInt)]
    access_flags: i64,
    #[diesel(sql_type = Text)]
    source: String,
}

impl<DB: Backend> Selectable<DB> for MethodSpecRow {
    type SelectExpression = (
        classes::id,
        classes::name,
        methods::id,
        methods::name,
        methods::args,
        methods::ret,
        methods::access_flags,
        sources::name,
    );

    fn construct_selection() -> Self::SelectExpression {
        (
            classes::id,
            classes::name,
            methods::id,
            methods::name,
            methods::args,
            methods::ret,
            methods::access_flags,
            sources::name,
        )
    }
}

impl From<MethodSpecRow> for MethodSpec {
    fn from(value: MethodSpecRow) -> Self {
        MethodSpec {
            class_id: value.class_id,
            class: value.class.into(),
            name: value.name,
            id: value.id,
            signature: value.args,
            ret: value.ret,
            access_flags: AccessFlag::from_bits_truncate(value.access_flags as u64),
            source: value.source,
        }
    }
}

/// One route, as the JSON array of method ids that [GraphDatabase::find_callers_from] builds
///
/// Only the ids come back from the query. The specs are fetched separately because the same
/// method appears in many routes.
#[derive(QueryableByName, Debug)]
struct RouteRow {
    #[diesel(sql_type = Text)]
    path: String,
}

impl RouteRow {
    fn method_ids(&self) -> Result<Vec<MethodId>> {
        let ids: Vec<i32> = serde_json::from_str(&self.path)
            .map_err(|e| Error::Generic(format!("bad route {}: {}", self.path, e)))?;
        Ok(ids.into_iter().map(MethodId::new).collect())
    }
}

#[derive(Queryable, QueryableByName, Debug)]
struct ChildClassRow {
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    access_flags: i64,
}

impl<DB: Backend> Selectable<DB> for ChildClassRow {
    type SelectExpression = (sources::name, classes::name, classes::access_flags);

    fn construct_selection() -> Self::SelectExpression {
        (sources::name, classes::name, classes::access_flags)
    }
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

fn to_json_array<T: DatabaseId>(items: &[T]) -> String {
    format!("[{}]", items.iter().map(|it| it.id().to_string()).join(","))
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
        let url = get_db_url(context);
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
                None,
            );

            macro_rules! path {
                ($({ $($name:ident: $val:expr),+ }),*) => {{

                    // The ids are not part of MethodSpec equality
                    let path = vec![$(
                            MethodSpec {
                                id: MethodId::new(1),
                                class_id: ClassId::new(1),
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
    fn test_find_callers_from(tmp_context: TestContext) {
        db_test(&tmp_context, |db| {
            let target_class = ClassName::from("Lbl/bl;");
            let target = MethodSearch::new(
                MethodSearchParams::ByFullSpec {
                    class: &target_class,
                    name: "by",
                    signature: "JLjava/lang/String;J",
                },
                None,
                None,
            );

            macro_rules! path {
                ($({ $($name:ident: $val:expr),+ }),*) => {{
                    // The ids are not part of MethodSpec equality
                    let path = vec![$(
                            MethodSpec {
                                id: MethodId::new(1),
                                class_id: ClassId::new(1),
                        $(
                                $name: $val.into()
                        ),+,
                                access_flags: AccessFlag::PUBLIC,
                            }
                    ),*];
                    MethodCallPath { path }
                }};
            }

            macro_rules! entries {
                ($class:expr, $name:expr, $sig:expr, $src:expr) => {{
                    let class = ClassName::from($class);
                    let search = MethodSearch::new(
                        MethodSearchParams::ByFullSpec {
                            class: &class,
                            name: $name,
                            signature: $sig,
                        },
                        $src,
                        None,
                    );
                    db.get_method_ids(&search).expect("get_method_ids")
                }};
            }

            // Reaches the target one hop away, so the route covers the method in between
            let entry = entries!("Lal/al;", "fi", "IZLjava/lang/String;", None);
            assert_eq!(
                db.find_callers_from(&target, &entry)
                    .expect("find_callers_from"),
                vec![path!(
                    {class: "al.al", name: "fi", signature: "IZLjava/lang/String;", source: "C", ret: "C"},
                    {class: "bs.bs", name: "fe", signature: "J", source: "framework", ret: "C"}
                )]
            );

            // An entry that calls the target itself is a route of one
            let entry = entries!("Lbs/bs;", "fe", "J", None);
            assert_eq!(
                db.find_callers_from(&target, &entry)
                    .expect("find_callers_from"),
                vec![path!(
                    {class: "bs.bs", name: "fe", signature: "J", source: "framework", ret: "C"}
                )]
            );

            // The same signature in a source that can't reach the target finds nothing
            let entry = entries!("Lax/ax;", "ds", "FIJ", Some("D"));
            assert!(db
                .find_callers_from(&target, &entry)
                .expect("find_callers_from")
                .is_empty());

            // No entries means there is nothing to search from
            assert!(db
                .find_callers_from(&target, &[])
                .expect("find_callers_from")
                .is_empty());
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
                    let mids: Vec<MethodId> = db.get_method_ids(&MethodSearch::new(MethodSearchParams::$sel { $($name: $val),+ }, $src, None)).expect("get_method_ids call failed");
                    for id in [$(MethodId::new($expected as i32)),*] {
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
