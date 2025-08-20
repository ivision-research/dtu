use std::collections::BTreeMap;

use cozo::ScriptMutability;

use crate::{utils::path_must_str, Context};

use super::CozoGraphDatabase;
use super::{GraphDatabaseInternal, LoadCSVKind};

type Result<T> = super::Result<T>;

impl CozoGraphDatabase {
    fn has_at_least_one(&self, col: &str, rel: &str, source: &str) -> bool {
        let script = format!(
            r#"
?[count({})] := *{}{{ {}, source: $source }}

:limit 1
"#,
            col, rel, col
        );

        let mut params = BTreeMap::new();
        params.insert(String::from("source"), source.into());

        #[cfg(feature = "trace_db")]
        log::trace!("{}", script);

        let named_rows = match self
            .db
            .run_script(&script, params, ScriptMutability::Immutable)
        {
            Err(_e) => return false,
            Ok(v) => v,
        };

        if named_rows.rows.len() < 1 {
            return false;
        }

        let row = &named_rows.rows[0];

        if row.len() < 1 {
            return false;
        }

        match row[0].get_int() {
            None => false,
            Some(v) => v > 0,
        }
    }

    fn has_classes_from(&self, source: &str) -> bool {
        self.has_at_least_one("name", "classes", source)
    }
    fn has_supers_from(&self, source: &str) -> bool {
        self.has_at_least_one("class", "supers", source)
    }
    fn has_impls_from(&self, source: &str) -> bool {
        self.has_at_least_one("class", "interfaces", source)
    }
    fn has_methods_from(&self, source: &str) -> bool {
        self.has_at_least_one("class", "methods", source)
    }
    fn has_calls_from(&self, source: &str) -> bool {
        self.has_at_least_one("from", "calls", source)
    }
}

impl GraphDatabaseInternal for CozoGraphDatabase {
    fn should_load_csv(&self, source: &str, csv: LoadCSVKind) -> bool {
        !match csv {
            LoadCSVKind::Classes => self.has_classes_from(source),
            LoadCSVKind::Impls => self.has_impls_from(source),
            LoadCSVKind::Supers => self.has_supers_from(source),
            LoadCSVKind::Methods => self.has_methods_from(source),
            LoadCSVKind::Calls => self.has_calls_from(source),
        }
    }

    fn load_classes_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let abs = ctx.get_graph_import_dir()?.join(path);
        let script = format!(
            r#"
csv[name, access_flags] <~ CsvReader(
    types: ['String', 'Int'],
    url: 'file://{}',
    has_headers: false
)

?[name, access_flags, source] := csv[name, access_flags], source = '{}'

:put classes {{ name, source => access_flags }}
"#,
            path_must_str(&abs),
            source
        );
        self.db.run_default(&script)?;
        Ok(())
    }

    fn load_supers_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let abs = ctx.get_graph_import_dir()?.join(path);
        // Load the new data and check if there are any of the supers don't already
        // exist in the classes relation. If they're not there, ensure we add
        // them, but we don't have much to say about access_flags besides
        // that it must be public
        let script = format!(
            r#"
{{
    csv[class, parent] <~ CsvReader(
        types: ['String', 'String'],
        url: 'file://{}',
        has_headers: false
    )


    ?[class, parent, source] := csv[class, parent], source = '{}'

    :put supers {{ class, parent, source }}
}}
{{
    new[name, access_flags] := *supers{{parent: name}}, not *classes{{name}}, access_flags = 2
    ?[name, access_flags, source ] := new[name, access_flags], source = '{}'
    :put classes {{ name, source => access_flags }}
}}
"#,
            path_must_str(&abs),
            source,
            source
        );
        self.db.run_default(&script)?;
        Ok(())
    }

    fn load_impls_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let abs = ctx.get_graph_import_dir()?.join(path);
        // Load the new data and check if any of the interfaces don't already
        // exist in the classes relation. If they're not there, ensure we add
        // them with access_flags saying that they're a public interface
        let script = format!(
            r#"
{{
    csv[class, interface] <~ CsvReader(
        types: ['String', 'String'],
        url: 'file://{}',
        has_headers: false
    )

    ?[class, interface, source] := csv[class, interface], source = '{}'

    :put interfaces {{ class, interface, source }}
}}
{{
    new[name, access_flags] := *interfaces{{interface: name}}, not *classes{{name}}, access_flags = 32770
    ?[name, access_flags, source ] := new[name, access_flags], source = '{}'
    :put classes {{ name, source => access_flags }}
}}
"#,
            path_must_str(&abs),
            source,
            source
        );
        self.db.run_default(&script)?;
        Ok(())
    }

    fn load_methods_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let abs = ctx.get_graph_import_dir()?.join(path);
        let script = format!(
            r#"
csv[class, name, sig, ret, access_flags] <~ CsvReader(
    types: ['String', 'String', 'String', 'String', 'Int'],
    url: 'file://{}',
    has_headers: false
)

?[class, name, sig, ret, source, access_flags] := csv[class, name, sig, ret, access_flags], source = '{}'

:put methods {{ class, name, sig, source => ret, access_flags }}
"#,
            path_must_str(&abs),
            source
        );
        self.db.run_default(&script)?;
        Ok(())
    }

    fn load_calls_csv(&self, ctx: &dyn Context, path: &str, source: &str) -> Result<()> {
        let abs = ctx.get_graph_import_dir()?.join(path);
        let script = format!(
            r#"
csv[from, to] <~ LoadCallsCsv(file: '{}')
?[from, to, source] := csv[from, to], source = '{}'
:put calls {{from, to, source}}
"#,
            path_must_str(&abs),
            source
        );
        self.db.run_default(&script)?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::testing::{tmp_context, TestContext};
    use rstest::*;

    #[fixture]
    #[once]
    fn testdb(tmp_context: TestContext) -> CozoGraphDatabase {
        let db = CozoGraphDatabase::new(&tmp_context).expect("failed to get database");
        db.db
            .run_default(super::super::cozodb::test::TEST_DATA_SCRIPT)
            .expect("failed to insert test data");
        db
    }

    #[rstest]
    fn test_has_classes_from(testdb: &CozoGraphDatabase) {
        assert_eq!(testdb.has_classes_from("framework"), true);
        assert_eq!(testdb.has_classes_from("apk"), true);
        assert_eq!(testdb.has_classes_from("cnn"), false);
    }

    #[rstest]
    fn test_has_impls_from(testdb: &CozoGraphDatabase) {
        assert_eq!(testdb.has_impls_from("framework"), true);
        assert_eq!(testdb.has_impls_from("apk"), true);
        assert_eq!(testdb.has_impls_from("cnn"), false);
    }

    #[rstest]
    fn test_has_methods_from(testdb: &CozoGraphDatabase) {
        assert_eq!(testdb.has_methods_from("framework"), true);
        assert_eq!(testdb.has_methods_from("apk"), true);
        assert_eq!(testdb.has_methods_from("cnn"), false);
    }

    #[rstest]
    fn test_has_calls_from(testdb: &CozoGraphDatabase) {
        assert_eq!(testdb.has_calls_from("framework"), true);
        assert_eq!(testdb.has_calls_from("apk"), true);
        assert_eq!(testdb.has_calls_from("cnn"), false);
    }
    #[rstest]
    fn test_has_supers_from(testdb: &CozoGraphDatabase) {
        assert_eq!(testdb.has_supers_from("framework"), true);
        assert_eq!(testdb.has_supers_from("apk"), true);
        assert_eq!(testdb.has_supers_from("cnn"), false);
    }
}
