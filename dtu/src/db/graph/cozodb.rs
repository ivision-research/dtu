use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;

use crate::db::graph::db::{Error, Result};
use crate::db::graph::models::{ClassCallPath, MethodMeta};
use crate::db::graph::{ClassMeta, GraphDatabase};
use crate::utils::ClassName;
use crate::Context;
use cozo::{DataValue, DbInstance, ScriptMutability};
use smalisa::AccessFlag;

use super::cozoutils::{FindPathsDFS, FindReachableDFS, LoadCallsCsv};
use super::models::{ClassSourceCallPath, MethodCallSearch};

/// Implements a [GraphDatabase] backed by CozoDB
pub struct CozoGraphDatabase {
    pub(super) db: DbInstance,
}

impl From<cozo::Error> for Error {
    fn from(err: cozo::Error) -> Error {
        Error::Generic(err.to_string())
    }
}

impl CozoGraphDatabase {
    #[cfg(test)]
    #[allow(unused_variables)]
    fn new_impl(ctx: &dyn Context) -> Result<Self> {
        let db = match DbInstance::new("mem", "", "") {
            Err(e) => return Err(Error::Generic(e.to_string())),
            Ok(v) => v,
        };

        let cozo = Self { db };
        cozo.setup()?;
        Ok(cozo)
    }

    #[cfg(not(test))]
    fn new_impl(ctx: &dyn Context) -> Result<Self> {
        let mut path = ctx.get_output_dir_child("cozo")?;
        crate::utils::ensure_dir_exists(&path)?;
        path.push("device.rdb");

        log::debug!(
            "Opening cozo database at {}",
            crate::utils::path_must_str(&path)
        );

        let needs_setup = !path.exists();

        let db = match DbInstance::new("rocksdb", &path, "") {
            Err(e) => return Err(Error::Generic(e.to_string())),
            Ok(v) => v,
        };

        log::trace!("cozo database opened");

        let cozo = Self { db };

        if needs_setup {
            log::debug!("Setting up the cozo database...");
            cozo.setup()?;
        }

        Ok(cozo)
    }

    /// Get a new Cozo database for the given source
    ///
    /// If the backing storage file doesn't exist, it is created and set up
    /// with the appropriate relations, but will be empty.
    pub fn new(ctx: &dyn Context) -> Result<Self> {
        let db = Self::new_impl(ctx)?;

        db.db
            .register_fixed_rule("FindPathsDFS".into(), FindPathsDFS {})?;
        db.db
            .register_fixed_rule("LoadCallsCsv".into(), LoadCallsCsv {})?;
        db.db
            .register_fixed_rule("FindReachableDFS".into(), FindReachableDFS {})?;

        Ok(db)
    }

    fn setup(&self) -> Result<()> {
        log::debug!("Setting up cozo database");
        const CLASSES_RELATION: &'static str = r#":create classes {
    name: String,
    source: String,
    =>
    access_flags: Int,
}"#;
        const CLASSES_INDEX: &'static str = "::index create classes:source {source}";

        const PARENTS_RELATION: &'static str = r#":create supers {
    class: String,
    parent: String,
    source: String,
}
"#;

        const PARENTS_INDEX: &'static str = "::index create supers:parent {parent}";

        const IMPLS_RELATION: &'static str = r#":create interfaces {
    class: String,
    interface: String,
    source: String,
}
"#;

        const IMPLS_INDEX: &'static str = "::index create interfaces:interface {interface}";

        const METHODS_RELATION: &'static str = r#":create methods {
    class: String,
    name: String,
    sig: String,
    source: String,
    =>
    ret: String,
    access_flags: Int,
}"#;
        const METHODS_SOURCE_INDEX: &'static str =
            "::index create methods:source {source, ret, access_flags}";

        const CALLS_RELATION: &'static str = r#":create calls {
    to: String,
    from: String,
    source: String,
}"#;

        const CALLS_SOURCE_IDX: &'static str = "::index create calls:source {source}";
        const CALLS_TO_IDX: &'static str = "::index create calls:to {to}";

        const SCRIPTS: &[&'static str] = &[
            CLASSES_RELATION,
            CLASSES_INDEX,
            PARENTS_RELATION,
            PARENTS_INDEX,
            IMPLS_RELATION,
            IMPLS_INDEX,
            METHODS_RELATION,
            METHODS_SOURCE_INDEX,
            CALLS_RELATION,
            CALLS_SOURCE_IDX,
            CALLS_TO_IDX,
        ];

        for script in SCRIPTS {
            log::trace!("Running:\n{}", script);
            self.db.run_default(*script)?;
        }

        Ok(())
    }

    fn run_bound_script<S>(
        &self,
        script: &str,
        setup: S,
        mutability: ScriptMutability,
    ) -> Result<()>
    where
        S: FnOnce(&mut BTreeMap<String, DataValue>),
    {
        #[cfg(feature = "trace_db")]
        log::trace!("{}", script);
        let mut params = BTreeMap::new();
        setup(&mut params);

        self.db.run_script(script, params, mutability)?;
        Ok(())
    }

    #[inline]
    pub fn run_mutable_bound_script<S>(&self, script: &str, setup: S) -> Result<()>
    where
        S: FnOnce(&mut BTreeMap<String, DataValue>),
    {
        self.run_bound_script(script, setup, ScriptMutability::Mutable)
    }

    #[inline]
    pub fn run_readonly_bound_script<S>(&self, script: &str, setup: S) -> Result<()>
    where
        S: FnOnce(&mut BTreeMap<String, DataValue>),
    {
        self.run_bound_script(script, setup, ScriptMutability::Immutable)
    }

    pub fn execute_bound_script<R, S>(&self, script: &str, mut on_row: R, setup: S) -> Result<()>
    where
        R: FnMut(Vec<DataValue>) -> Result<()>,
        S: FnOnce(&mut BTreeMap<String, DataValue>),
    {
        #[cfg(feature = "trace_db")]
        {
            #[cfg(test)]
            eprintln!("{}", script);
            #[cfg(not(test))]
            log::trace!("{}", script);
        }

        let mut params = BTreeMap::new();
        setup(&mut params);

        let named_rows = self
            .db
            .run_script(script, params, ScriptMutability::Immutable)?;

        for r in named_rows.rows {
            on_row(r)?;
        }

        let mut next = match named_rows.next {
            Some(v) => v,
            None => return Ok(()),
        };

        loop {
            for r in next.rows {
                on_row(r)?;
            }
            next = match next.next {
                Some(v) => v,
                None => break,
            };
        }
        Ok(())
    }
}

impl GraphDatabase for CozoGraphDatabase {
    fn initialize(&self) -> Result<()> {
        Ok(())
    }

    fn optimize(&self) -> Result<()> {
        self.db.run_default("::compact")?;
        Ok(())
    }

    fn eval(&self, script: &str, writer: &mut dyn Write) -> Result<()> {
        let res = self.db.run_default(script)?;
        for row in res.rows {
            let count = row.len();
            for (i, val) in row.iter().enumerate() {
                writer.write_all(val.to_string().as_bytes())?;
                if i < count - 1 {
                    writer.write_all(b", ")?;
                }
            }
            writer.write(&[b'\n'])?;
        }
        Ok(())
    }

    #[cfg(feature = "setup")]
    fn add_directory(
        &self,
        ctx: &dyn Context,
        opts: super::AddDirectoryOptions,
        monitor: &dyn crate::tasks::EventMonitor<super::SetupEvent>,
        cancel: &crate::tasks::TaskCancelCheck,
    ) -> Result<()> {
        let task = super::AddDirTask {
            ctx,
            cancel,
            opts,
            monitor,
            graph: self,
        };

        task.run()?;
        Ok(())
    }

    fn find_outgoing_calls(
        &self,
        from: &MethodMeta,
        source: &str,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<ClassCallPath>> {
        // TODO: limit need sto be put into FindReachableDFS ... it isn't helping without that

        let mut script = if depth == 1 {
            "?[from, to, path] := *calls:source{from, to, source}, from = $from, path = [$from, to], source = $source".into()
        } else {
            format!(
                r#"source_filtered[from, to] := *calls:source{{from, to, source: $source}}
start[from] := *calls:source{{from, source: $source}}, from = $from
?[from, to, path] <~ FindReachableDFS(source_filtered[from, to], start[], depth: {})
"#,
                depth
            )
        };

        if let Some(limit) = limit {
            if limit > 0 {
                script.push_str(&format!("\n:limit {}", limit));
            }
        }

        let mut results = Vec::new();

        self.execute_bound_script(
            &script,
            |row| {
                let from = MethodMeta::from_smali(row[0].get_str().unwrap())?;
                let path = row[2].get_slice().unwrap();
                let mut call_path = Vec::with_capacity(path.len());

                for p in path {
                    let mm = MethodMeta::from_smali(p.get_str().unwrap())?;
                    call_path.push(mm);
                }
                results.push(ClassCallPath {
                    class: from.class,
                    path: call_path,
                });

                Ok(())
            },
            |p| {
                let as_str = if from.ret.is_some() {
                    // Clear the return value
                    MethodMeta {
                        class: from.class.clone(),
                        name: from.name.clone(),
                        signature: from.signature.clone(),
                        ret: None,
                        access_flags: AccessFlag::UNSET,
                    }
                    .as_smali()
                } else {
                    from.as_smali()
                };

                p.insert("source".into(), source.into());
                p.insert("from".into(), as_str.into());
            },
        )?;

        Ok(results)
    }

    fn get_classes_for(&self, source: &str) -> Result<Vec<ClassName>> {
        let script = "?[name] := *classes:source{name, source: $source}";
        let mut result = Vec::new();
        self.execute_bound_script(
            script,
            |row| {
                if row.len() != 1 {
                    return Err(Error::Generic("invalid result for get_classes_for".into()));
                }
                let class = row[0].get_str().unwrap();
                result.push(ClassName::from(class));
                Ok(())
            },
            |p| {
                p.insert("source".into(), source.into());
            },
        )?;

        Ok(result)
    }

    fn get_methods_for(&self, source: &str) -> Result<Vec<MethodMeta>> {
        let script =
            "?[class, name, sig, ret, access_flags] := *methods:source{class, name, sig, ret, access_flags, source: $source}";
        let mut result = Vec::new();
        self.execute_bound_script(
            script,
            |row| {
                if row.len() != 5 {
                    return Err(Error::Generic("invalid result for get_methods_for".into()));
                }
                let class = ClassName::from(row[0].get_str().unwrap());
                let name: String = row[1].get_str().unwrap().into();
                let signature: String = row[2].get_str().unwrap().into();
                let ret: String = row[3].get_str().unwrap().into();
                let access_flags = AccessFlag::from_bits_truncate(row[4].get_int().unwrap() as u64);
                result.push(MethodMeta {
                    class,
                    name,
                    signature,
                    ret: Some(ret),
                    access_flags,
                });
                Ok(())
            },
            |p| {
                p.insert("source".into(), source.into());
            },
        )?;

        Ok(result)
    }

    fn find_child_classes_of(
        &self,
        parent: &ClassName,
        source: Option<&str>,
    ) -> Result<Vec<ClassMeta>> {
        let script = if source.is_some() {
            r#"
child[class] := *supers{class, parent: $parent, source: $source}
child[class] := child[parent], *supers{class, parent, source: $source}
?[name, access_flags, source] := child[name], *classes{name, access_flags, source}
"#
        } else {
            r#"
child[class] := *supers{class, parent: $parent}
child[class] := child[parent], *supers{class, parent}
?[name, access_flags, source] := child[name], *classes{name, access_flags, source}
"#
        };
        let mut res = Vec::new();

        self.execute_bound_script(
            script,
            |row| {
                if row.len() != 3 {
                    return Err(Error::Generic(
                        "invalid result for find_child_classes_of".into(),
                    ));
                }
                let class = row[0].get_str().unwrap();
                let flags = row[1].get_int().unwrap();
                let source = row[2].get_str().unwrap();
                res.push(ClassMeta {
                    name: ClassName::from(class),
                    access_flags: AccessFlag::from_bits_truncate(flags as u64),
                    source: source.into(),
                });
                Ok(())
            },
            |params| {
                params.insert("parent".into(), parent.get_smali_name().as_ref().into());
                if let Some(src) = source {
                    params.insert("source".into(), src.into());
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
        let script = if source.is_none() {
            r#"
children[child, parent] := *supers{class: child, parent: parent}
children[child, parent] := children[q, parent], *supers{class: child, parent: q}

impls[class] := *interfaces{class, interface: $interface}

?[name, access_flags, source] := impls[q], children[name, q], *classes{name, access_flags, source}
?[name, access_flags, source] := impls[name], *classes{name, access_flags, source}
"#
        } else {
            r#"
children[child, parent] := *supers{class: child, parent: parent, source: $source}
children[child, parent] := children[q, parent], *supers{class: child, parent: q, source: $source}

impls[class] := *interfaces{class, interface: $interface, source: $source}

?[name, access_flags, source] := impls[q], children[name, q], *classes{name, access_flags, source}
?[name, access_flags, source] := impls[name], *classes{name, access_flags, source}
"#
        };
        let mut res = Vec::new();

        self.execute_bound_script(
            script,
            |row| {
                if row.len() != 3 {
                    return Err(Error::Generic(
                        "invalid result for find_classes_implementing".into(),
                    ));
                }
                let class = row[0].get_str().unwrap();
                let flags = row[1].get_int().unwrap();
                let source = row[2].get_str().unwrap();
                res.push(ClassMeta {
                    name: ClassName::from(class),
                    access_flags: AccessFlag::from_bits_truncate(flags as u64),
                    source: source.into(),
                });
                Ok(())
            },
            |params| {
                params.insert("interface".into(), iface.get_smali_name().as_ref().into());
                if let Some(src) = source {
                    params.insert("source".into(), src.into());
                }
            },
        )?;

        Ok(res)
    }

    fn get_all_sources(&self) -> Result<BTreeSet<String>> {
        let script = "?[source] := *classes{source}";
        Ok(self
            .db
            .run_default(script)?
            .into_iter()
            .filter_map(|it| it.get(0)?.get_str().map(String::from))
            .collect::<BTreeSet<String>>())
    }

    fn find_callers(
        &self,
        method: &MethodCallSearch,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<ClassSourceCallPath>> {
        let mut params = BTreeMap::new();
        let mut results = Vec::new();
        let script = method.create_script(&mut params, depth, limit)?;

        #[cfg(feature = "trace_db")]
        {
            #[cfg(test)]
            eprintln!("{}", script);
            #[cfg(not(test))]
            log::trace!("{}", script);
        }

        let result = self
            .db
            .run_script(&script, params, ScriptMutability::Immutable)?;

        for r in result {
            let source = String::from(r[0].get_str().unwrap());
            let from = MethodMeta::from_smali(r[1].get_str().unwrap())?;
            //let to = &r[2];
            let path = r[3].get_slice().unwrap();

            let mut call_path = Vec::with_capacity(path.len());

            for p in path {
                let mm = MethodMeta::from_smali(p.get_str().unwrap())?;
                call_path.push(mm);
            }

            results.push(ClassSourceCallPath {
                source,
                class: from.class,
                path: call_path,
            });
        }

        Ok(results)
    }

    fn wipe(&self, ctx: &dyn Context) -> Result<()> {
        let mut path = ctx.get_output_dir_child("cozo")?;
        path.push("device.rdb");
        if !path.exists() {
            return Ok(());
        }
        fs::remove_dir_all(&path)?;
        Ok(())
    }

    fn remove_source(&self, source: &str) -> Result<()> {
        self.run_readonly_bound_script(":rm classes{source: $source}", |q| {
            q.insert("source".into(), source.into());
        })?;
        self.run_readonly_bound_script(":rm interfaces{source: $source}", |q| {
            q.insert("source".into(), source.into());
        })?;
        self.run_readonly_bound_script(":rm methods{source: $source}", |q| {
            q.insert("source".into(), source.into());
        })?;
        self.run_readonly_bound_script(":rm calls{source: $source}", |q| {
            q.insert("source".into(), source.into());
        })?;
        self.run_readonly_bound_script(":rm classes{source: $source}", |q| {
            q.insert("source".into(), source.into());
        })?;
        Ok(())
    }
}

impl<'a> MethodCallSearch<'a> {
    fn create_script_unit_depth(
        &self,
        params: &mut BTreeMap<String, DataValue>,
        limit: Option<usize>,
    ) -> Result<String> {
        if !self.has_from() {
            return self.create_script_unit_depth_no_from(params, limit);
        }
        // TODO We can do more here, but this is fine for now
        self.create_script_nonunit_depth(params, 1, limit)
    }

    fn create_script_unit_depth_no_from(
        &self,
        params: &mut BTreeMap<String, DataValue>,
        limit: Option<usize>,
    ) -> Result<String> {
        if let Some(class) = self.target_class {
            return self.create_script_unit_depth_no_from_full_to(class, params, limit);
        }

        let to = format!(";->{}({})", self.target_method, self.target_method_sig);

        params.insert("to".into(), to.into());

        let mut script = String::from(if let Some(src) = self.source {
            params.insert("source".into(), src.into());
            r#"
G[from, to] := *calls:source{from, to, source: $source}
sources[from] := G[from, to], ends_with(to, $to)
?[source, from, to, path] := sources[from], to = $to, source = $source, path = [from, to]
"#
        } else {
            r#"
sources[from, source] := *calls:to{from, to, source}, ends_with(to, $to)
?[source, from, to, path] := sources[from, source], to = $to, path = [from, to]
"#
        });

        if let Some(limit) = limit {
            script.push_str(&format!("\n:limit {}", limit));
        }

        Ok(script)
    }

    fn create_script_unit_depth_no_from_full_to(
        &self,
        class: &ClassName,
        params: &mut BTreeMap<String, DataValue>,
        limit: Option<usize>,
    ) -> Result<String> {
        let to = format!(
            "{}->{}({})",
            class.get_smali_name(),
            self.target_method,
            self.target_method_sig
        );

        params.insert("to".into(), to.into());

        let mut script = String::from(if let Some(src) = self.source {
            params.insert("source".into(), src.into());
            r#"
G[from, to] := *calls:source{from, to, source: $source}
sources[from] := G[from, $to]
?[source, from, to, path] := sources[from], to = $to, source = $source, path = [from, to]
"#
        } else {
            r#"
sources[from, source] := *calls:to{from, to: $to, source}
?[source, from, to, path] := sources[from, source], to = $to, path = [from, to]
"#
        });

        if let Some(limit) = limit {
            script.push_str(&format!("\n:limit {}", limit));
        }

        Ok(script)
    }

    fn create_script(
        &self,
        params: &mut BTreeMap<String, DataValue>,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<String> {
        if depth == 1 {
            return self.create_script_unit_depth(params, limit);
        }
        self.create_script_nonunit_depth(params, depth, limit)
    }

    fn create_script_nonunit_depth(
        &self,
        params: &mut BTreeMap<String, DataValue>,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<String> {
        let start = self.get_start(params)?;
        let end = self.get_end(params)?;

        let graph = if let Some(src) = self.source {
            params.insert("source".into(), src.into());
            "G[from, to, source] := *calls:source{from, to, source: $source}, source = $source"
        } else {
            // Yikes this will likely EXPLODE, maybe we should come up with some sort of paging
            // mechanism here or something
            "G[from, to, source] := *calls{from, to, source}"
        };

        // TODO: limit need sto be put into FindPathsDFS ...

        let mut script = format!(
            "{}\n{}\n{}\n?[source, from, to, paths] <~ FindPathsDFS(G[from, to], start[], end[], depth: {})",
            graph, start, end, depth
        );

        if let Some(limit) = limit {
            if limit > 0 {
                script.push_str(&format!("\n:limit {}", limit));
            }
        }
        Ok(script)
    }

    fn get_condition(
        &self,
        key: &str,
        class: Option<&ClassName>,
        method: Option<&str>,
        sig: Option<&str>,
        params: &mut BTreeMap<String, DataValue>,
    ) -> Result<Option<String>> {
        let has_class = !class.is_none();
        let has_name = !method.is_none();
        let has_sig = !sig.is_none();

        if !(has_class || has_sig || has_name) {
            return Ok(None);
        }

        if has_name && !(has_class || has_sig) {
            return Err(Error::Generic(
                "invalid find_calls query: method name provided without class or signature".into(),
            ));
        }

        // All three is just a simple lookup
        if has_class && has_name && has_sig {
            params.insert(
                String::from(key),
                format!(
                    "{}->{}({})",
                    class.unwrap().get_smali_name(),
                    method.unwrap(),
                    sig.unwrap(),
                )
                .into(),
            );
            return Ok(Some(format!("{} = ${}", key, key)));
        }

        // Otherwise we'll use combinations of starts_with and ends_with
        if has_class {
            let cn = class.unwrap().get_smali_name();

            // class + name -> starts_with
            if has_name {
                params.insert(
                    String::from(key),
                    format!("{}->{}(", cn, method.unwrap()).into(),
                );
                return Ok(Some(format!("starts_with({}, ${})", key, key)));
            }

            // Just class name -> starts_with
            if !has_sig {
                params.insert(String::from(key), cn.as_ref().into());
                return Ok(Some(format!("starts_with({}, ${})", key, key)));
            }

            // Class and sig -> starts_with AND ends_with
            params.insert(format!("{}_start", key).into(), cn.as_ref().into());
            params.insert(
                format!("{}_end", key).into(),
                format!("({})", sig.unwrap()).into(),
            );
            return Ok(Some(format!(
                "and(starts_with({}, ${}_start), ends_with({}, ${}_end))",
                key, key, key, key
            )));
        } else if has_name {
            if !has_sig {
                return Err(Error::Generic(
                    "invalid find_calls query: method name provided without class or signature"
                        .into(),
                ));
            }
            // name + sig -> ends_with
            params.insert(
                String::from(key),
                format!("->{}({})", method.unwrap(), sig.unwrap()).into(),
            );
            return Ok(Some(format!("ends_with({}, ${})", key, key).into()));
        }

        // Just searching by signature... ok

        params.insert(String::from(key), format!("({})", sig.unwrap()).into());
        Ok(Some(format!("ends_with({}, ${})", key, key)))
    }

    fn get_intermediate_query(
        &self,
        is_from: bool,
        params: &mut BTreeMap<String, DataValue>,
    ) -> Result<String> {
        let (class, method, sig) = if is_from {
            (self.src_class, self.src_method_name, self.src_method_sig)
        } else {
            (
                self.target_class,
                Some(self.target_method),
                Some(self.target_method_sig),
            )
        };

        let mut s = String::from(if is_from {
            "start[from, source] := G[from, _, source]"
        } else {
            "end[to] := G[_, to, _]"
        });

        let suffix = self.get_condition(
            if is_from { "from" } else { "to" },
            class,
            method,
            sig,
            params,
        )?;

        if let Some(suffix) = suffix {
            s.push_str(", ");
            s.push_str(&suffix);
        }
        Ok(s)
    }

    fn get_start(&self, params: &mut BTreeMap<String, DataValue>) -> Result<String> {
        self.get_intermediate_query(true, params)
    }

    fn get_end(&self, params: &mut BTreeMap<String, DataValue>) -> Result<String> {
        self.get_intermediate_query(false, params)
    }

    #[allow(dead_code)]
    fn full_to(&self) -> bool {
        self.target_class.is_some()
    }

    fn has_from(&self) -> bool {
        self.src_class.is_some() || self.src_method_name.is_some()
    }
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;
    use crate::testing::{tmp_context, TestContext};
    use rstest::*;
    use std::collections::BTreeSet;

    #[fixture]
    #[once]
    fn testdb(tmp_context: TestContext) -> CozoGraphDatabase {
        let db = CozoGraphDatabase::new(&tmp_context).expect("failed to get database");
        db.db
            .run_default(TEST_DATA_SCRIPT)
            .expect("failed to insert test data");
        db
    }

    #[rstest]
    fn test_callers_raw(testdb: &CozoGraphDatabase) {
        let mut params = BTreeMap::new();
        params.insert("source".into(), "framework".into());
        params.insert("from".into(), "LA".into());
        params.insert("to".into(), "LT;->T()".into());

        let script = r#"source_filtered[from, to] := *calls{from, to, source: $source}
start[from, source] := source_filtered[from, _], starts_with(from, $from), source = $source
end[to] := source_filtered[_, to], to = $to

?[source, from, to, paths] <~ FindPathsDFS(source_filtered[from, to], start[], end[], depth: 3)"#;

        let _result = testdb
            .db
            .run_script(script, params, ScriptMutability::Immutable)
            .expect("failed to run");
    }

    #[rstest]
    fn test_callers_no_source_target_only(testdb: &CozoGraphDatabase) {
        let target_class = ClassName::from("LT;");
        let search = MethodCallSearch {
            target_method: "T",
            target_method_sig: "",
            target_class: Some(&target_class),
            src_class: None,
            src_method_name: None,
            src_method_sig: None,
            source: None,
        };

        let result = testdb
            .find_callers(&search, 2, None)
            .expect("failed to find callers");
        let expected = vec![
            ClassSourceCallPath {
                class: ClassName::from("LD;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LF;"),
                source: "apk".into(),
                path: vec![
                    MethodMeta::from_smali("LF;->F()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LQ;"),
                source: "apk".into(),
                path: vec![
                    MethodMeta::from_smali("LQ;->Q()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LC;"),
                source: "apk".into(),
                path: vec![
                    MethodMeta::from_smali("LC;->C()").unwrap(),
                    MethodMeta::from_smali("LF;->F()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LC;"),
                source: "apk".into(),
                path: vec![
                    MethodMeta::from_smali("LC;->C()").unwrap(),
                    MethodMeta::from_smali("LQ;->Q()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LB;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LB;->B()").unwrap(),
                    MethodMeta::from_smali("LF;->F()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LB;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
        ];
        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());
        assert_eq!(result_bt, expected_bt);
    }

    #[rstest]
    fn test_callers_src_method_only(testdb: &CozoGraphDatabase) {
        let target_class = ClassName::from("LT;");
        let search = MethodCallSearch {
            target_method: "T",
            target_method_sig: "",
            target_class: Some(&target_class),
            src_class: None,
            src_method_name: Some("A"),
            src_method_sig: Some("I"),
            source: Some("framework"),
        };

        let result = testdb
            .find_callers(&search, 3, None)
            .expect("failed to find callers");
        let expected = vec![
            ClassSourceCallPath {
                class: ClassName::from("LA;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LM;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LM;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
        ];

        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());
        assert_eq!(result_bt, expected_bt);
    }

    #[rstest]
    fn test_callers_src_class_only(testdb: &CozoGraphDatabase) {
        let src_class = ClassName::from("LA;");
        let target_class = ClassName::from("LT;");
        let search = MethodCallSearch {
            target_method: "T",
            target_method_sig: "",
            target_class: Some(&target_class),
            src_class: Some(&src_class),
            src_method_name: None,
            src_method_sig: None,
            source: Some("framework"),
        };

        let result = testdb
            .find_callers(&search, 3, None)
            .expect("failed to find callers");
        let expected = vec![
            ClassSourceCallPath {
                class: ClassName::from("LA;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
            ClassSourceCallPath {
                class: ClassName::from("LA;"),
                source: "framework".into(),
                path: vec![
                    MethodMeta::from_smali("LA;->Q()").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ],
            },
        ];
        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());
        assert_eq!(result_bt, expected_bt);
    }

    #[rstest]
    fn test_callers_full_spec(testdb: &CozoGraphDatabase) {
        let src_class = ClassName::from("LA;");
        let target_class = ClassName::from("LT;");
        let search = MethodCallSearch {
            target_method: "T",
            target_method_sig: "",
            target_class: Some(&target_class),
            src_class: Some(&src_class),
            src_method_name: Some("A"),
            src_method_sig: Some("I"),
            source: Some("framework"),
        };

        let result = testdb
            .find_callers(&search, 3, None)
            .expect("failed to find callers");
        assert_eq!(
            result,
            vec![ClassSourceCallPath {
                source: "framework".into(),
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                    MethodMeta::from_smali("LT;->T()").unwrap(),
                ]
            }]
        )
    }

    #[rstest]
    fn test_find_impls(testdb: &CozoGraphDatabase) {
        let iface = ClassName::from("Lparent/A;");
        assert_eq!(
            testdb
                .find_classes_implementing(&iface, None)
                .expect("failed to search for interfaces"),
            vec![]
        );

        let iface = ClassName::from("Liface/A;");

        let all_sources = testdb
            .find_classes_implementing(&iface, None)
            .expect("failed to get search for interfaces");
        assert_eq!(
            all_sources,
            vec![
                ClassMeta {
                    name: ClassName::from("class.A"),
                    access_flags: AccessFlag::PUBLIC,
                    source: "framework".into(),
                },
                ClassMeta {
                    name: ClassName::from("class.E"),
                    access_flags: AccessFlag::PUBLIC,
                    source: "framework".into(),
                },
                ClassMeta {
                    name: ClassName::from("class.G"),
                    access_flags: AccessFlag::PUBLIC,
                    source: "apk".into(),
                },
                ClassMeta {
                    name: ClassName::from("class.J"),
                    access_flags: AccessFlag::PUBLIC,
                    source: "apk".into(),
                },
            ]
        );

        let apk_source = testdb
            .find_classes_implementing(&iface, Some("apk"))
            .expect("failed to get child classes");
        assert_eq!(
            apk_source,
            vec![
                ClassMeta {
                    name: ClassName::from("class.G"),
                    access_flags: AccessFlag::PUBLIC,
                    source: "apk".into(),
                },
                ClassMeta {
                    name: ClassName::from("class.J"),
                    access_flags: AccessFlag::PUBLIC,
                    source: "apk".into(),
                },
            ]
        );
    }

    #[rstest]
    fn test_find_children_no_source(testdb: &CozoGraphDatabase) {
        let class = ClassName::from("Lparent/A;");
        let result = testdb
            .find_child_classes_of(&class, None)
            .expect("failed to get child classes");

        let expected = vec![
            ClassMeta {
                name: ClassName::from("class.A"),
                access_flags: AccessFlag::PUBLIC,
                source: "framework".into(),
            },
            ClassMeta {
                name: ClassName::from("class.B"),
                access_flags: AccessFlag::PUBLIC,
                source: "framework".into(),
            },
            ClassMeta {
                name: ClassName::from("class.E"),
                access_flags: AccessFlag::PUBLIC,
                source: "framework".into(),
            },
            ClassMeta {
                name: ClassName::from("class.F"),
                access_flags: AccessFlag::PUBLIC,
                source: "apk".into(),
            },
            ClassMeta {
                name: ClassName::from("class.G"),
                access_flags: AccessFlag::PUBLIC,
                source: "apk".into(),
            },
            ClassMeta {
                name: ClassName::from("class.J"),
                access_flags: AccessFlag::PUBLIC,
                source: "apk".into(),
            },
        ];

        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());

        assert_eq!(result_bt, expected_bt);
    }

    #[rstest]
    fn test_find_children_source(testdb: &CozoGraphDatabase) {
        let class = ClassName::from("Lparent/A;");
        let result = testdb
            .find_child_classes_of(&class, Some("apk"))
            .expect("failed to get child classes");

        let expected = vec![
            ClassMeta {
                name: ClassName::from("class.F"),
                access_flags: AccessFlag::PUBLIC,
                source: "apk".into(),
            },
            ClassMeta {
                name: ClassName::from("class.G"),
                access_flags: AccessFlag::PUBLIC,
                source: "apk".into(),
            },
            ClassMeta {
                name: ClassName::from("class.J"),
                access_flags: AccessFlag::PUBLIC,
                source: "apk".into(),
            },
        ];

        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());

        assert_eq!(result_bt, expected_bt);
    }

    #[rstest]
    fn test_find_outgoing_calls_limit(testdb: &CozoGraphDatabase) {
        let source = "framework";
        let class = ClassName::from("LA;");
        let search = MethodMeta {
            class,
            name: "A".into(),
            signature: "I".into(),
            ret: None,
            access_flags: AccessFlag::UNSET,
        };

        let result = testdb
            .find_outgoing_calls(&search, source, 2, Some(1))
            .expect("failed to search for calls");

        assert_eq!(result.len(), 1);
    }

    #[rstest]
    fn test_find_outgoing_calls_depth_1(testdb: &CozoGraphDatabase) {
        let source = "framework";
        let class = ClassName::from("LA;");
        let search = MethodMeta {
            class,
            name: "A".into(),
            signature: "I".into(),
            ret: None,
            access_flags: AccessFlag::UNSET,
        };

        let result = testdb
            .find_outgoing_calls(&search, source, 1, None)
            .expect("failed to search for calls");

        let expected = vec![
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                ],
            },
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->C()").unwrap(),
                ],
            },
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->H()").unwrap(),
                ],
            },
        ];

        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());

        assert_eq!(result_bt, expected_bt);
    }

    #[rstest]
    fn test_find_outgoing_calls(testdb: &CozoGraphDatabase) {
        let source = "framework";
        let class = ClassName::from("LA;");
        let search = MethodMeta {
            class,
            name: "A".into(),
            signature: "I".into(),
            ret: None,
            access_flags: AccessFlag::UNSET,
        };

        let result = testdb
            .find_outgoing_calls(&search, source, 2, None)
            .expect("failed to search for calls");

        let expected = vec![
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                ],
            },
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->C()").unwrap(),
                ],
            },
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->H()").unwrap(),
                ],
            },
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LD;->D()").unwrap(),
                ],
            },
            ClassCallPath {
                class: ClassName::from("LA;"),
                path: vec![
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                    MethodMeta::from_smali("LB;->B(Ljava/lang/String;)").unwrap(),
                    MethodMeta::from_smali("LA;->A(I)").unwrap(),
                ],
            },
        ];

        let mut result_bt = BTreeSet::new();
        result_bt.extend(result.into_iter());
        let mut expected_bt = BTreeSet::new();
        expected_bt.extend(expected.into_iter());

        assert_eq!(result_bt, expected_bt);
    }

    pub(crate) const TEST_DATA_SCRIPT: &'static str = r#"
{
    ?[name, access_flags, source] <- [
        ["Lclass/A;", 2, "framework"],
        ["Lclass/B;", 2, "framework"],
        ["Lclass/C;", 2, "framework"],
        ["Lclass/D;", 2, "framework"],
        ["Lclass/E;", 2, "framework"],
        ["Liface/A;", 32770, "framework"],
        ["Liface/B;", 32770, "framework"],
        ["Lparent/A;", 2, "framework"],
        ["Lparent/B;", 2, "framework"],

        ["Lclass/F;", 2, "apk"],
        ["Lclass/G;", 2, "apk"],
        ["Lclass/H;", 2, "apk"],
        ["Lclass/I;", 2, "apk"],
        ["Lclass/J;", 2, "apk"],
        ["Liface/C;", 32770, "apk"],
        ["Liface/D;", 32770, "apk"],
        ["Lparent/C;", 2, "apk"],
        ["Lparent/D;", 2, "apk"],
    ]

    :put classes {name, source => access_flags}
}

{
    ?[class, parent, source] <- [
        ["Lclass/A;", "Lparent/A;", "framework"],
        ["Lclass/B;", "Lparent/A;", "framework"],
        ["Lclass/C;", "Lparent/B;", "framework"],
        ["Lclass/D;", "Lclass/C;", "framework"],
        ["Lclass/E;", "Lclass/A;", "framework"],

        ["Lclass/F;", "Lparent/A;", "apk"],
        ["Lclass/H;", "Lparent/C;", "apk"],
        ["Lclass/I;", "Lparent/C;", "apk"],
        ["Lclass/G;", "Lclass/F;", "apk"],
        ["Lclass/J;", "Lclass/G;", "apk"],
    ]

    :put supers { class, parent, source }
}

{
    ?[class, interface, source] <- [
        ["Lclass/D;", "Liface/B;", "framework"],
        ["Lclass/E;", "Liface/B;", "framework"],
        ["Lclass/A;", "Liface/A;", "framework"],

        ["Lclass/F;", "Liface/C;", "apk"],
        ["Lclass/G;", "Liface/A;", "apk"],
        ["Lclass/G;", "Liface/D;", "apk"],
        ["Lclass/I;", "Liface/D;", "apk"],
    ]

    :put interfaces { class, interface, source }
}

{
    ?[class, name, sig, ret, access_flags, source] <- [
        ["Lclass/D;", "method", "", "V", 2, "framework"],
        ["Lclass/C;", "method", "", "V", 2, "framework"],
        ["Lclass/D;", "a", "I", "V", 2, "framework"],

        ["Lclass/F;", "method", "", "V", 2, "apk"],
        ["Lclass/C;", "b", "Ljava/lang/String;", "V", 2, "apk"],
        ["Lclass/F;", "c", "", "V", 2, "apk"],
    ]

    :put methods { class, name, sig, source => ret, access_flags }
}

{
    ?[from, to, source] <- [
        ["LZ;->Z()", "LP;->P()", "apk"],

        ["LA;->A(I)", "LB;->B(Ljava/lang/String;)", "framework"],
        ["LA;->A(I)", "LB;->C()", "framework"],
        ["LA;->A(I)", "LB;->H()", "framework"],
        ["LA;->Q()", "LB;->B(Ljava/lang/String;)", "framework"],

        ["LM;->A(I)", "LB;->B(Ljava/lang/String;)", "framework"],

        ['LB;->B(Ljava/lang/String;)', 'LD;->D()', 'framework'],
        ['LB;->B(Ljava/lang/String;)', 'LA;->A(I)', 'framework'],
        ['LB;->B()', 'LF;->F()', 'framework'],

        ['LC;->C()', 'LE;->E()', 'apk'],
        ['LC;->C()', 'LF;->F()', 'apk'],
        ['LC;->C()', 'LQ;->Q()', 'apk'],

        ['LH;->H()', 'LB;->B()', 'framework'],

        ['LD;->D()', 'LT;->T()', 'framework'],
        
        ['LF;->F()', 'LT;->T()', 'apk'],

        ['LQ;->Q()', 'LT;->T()', 'apk'],
    ]

    :put calls { from, to, source }
}
"#;
}
