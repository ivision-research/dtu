use std::fmt::Display;
use std::path::Path;
use std::{borrow::Cow, path::PathBuf};
use toml::{Table, Value};

use crate::utils::{path_must_str, read_file};

#[derive(Debug)]
pub enum Error {
    InvalidType,
    MissingKey,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::InvalidType => "InvalidType",
                Self::MissingKey => "MissingKey",
            }
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct ConfigMap<'c> {
    path: &'c Path,
    name: Option<Cow<'c, str>>,
    table: &'c Table,
}

#[derive(Clone)]
pub struct Config {
    path: PathBuf,
    base: Table,
}

impl Config {
    pub fn parse(source: &Path) -> crate::Result<Self> {
        let as_str = read_file(source)?;

        let path = PathBuf::from(source);

        let base: Table = match toml::from_str(&as_str) {
            Ok(v) => v,
            Err(e) => return Err(crate::Error::new_cfg(source, &e)),
        };
        Ok(Self { base, path })
    }

    pub fn invalid_error(&self, msg: String) -> crate::Error {
        crate::Error::InvalidConfig(path_must_str(&self.path).into(), msg)
    }

    pub fn get_map(&self) -> ConfigMap {
        ConfigMap {
            name: None,
            path: &self.path,
            table: &self.base,
        }
    }
}

pub fn parse_config<R, F>(file: &Path, f: F) -> crate::Result<R>
where
    F: FnOnce(&ConfigMap) -> crate::Result<R>,
{
    let as_str = read_file(file)?;

    let table: Table = match toml::from_str(&as_str) {
        Ok(v) => v,
        Err(e) => return Err(crate::Error::new_cfg(file, &e)),
    };

    let base = ConfigMap {
        name: None,
        path: file,
        table: &table,
    };

    let res = f(&base);

    res
}

impl<'c> ConfigMap<'c> {
    fn get_full_path<'a>(&'a self) -> Option<&'a str> {
        self.name.as_ref().map(|it| it.as_ref())
    }

    fn key_path<'a>(&self, key: &'a str) -> Cow<'a, str> {
        match self.get_full_path() {
            None => Cow::Borrowed(key),
            Some(v) => Cow::Owned(format!("{}.{}", v, key)),
        }
    }

    /// Helper to create a crate::Error for a missing key
    pub fn missing_key(&self, key: &str) -> crate::Error {
        let path = self.key_path(key);
        crate::Error::InvalidConfig(
            path_must_str(self.path).into(),
            format!("missing key: {}", path),
        )
    }

    /// Helper to create a crate::Error for an invalid key
    pub fn invalid_key(&self, key: &str, expected: &str) -> crate::Error {
        let path = self.key_path(key);
        crate::Error::InvalidConfig(
            path_must_str(self.path).into(),
            format!(
                "invalid value for key: {} (expected type: {})",
                path, expected
            ),
        )
    }

    pub fn has(&self, key: &str) -> bool {
        self.table.contains_key(key)
    }

    fn get(&self, key: &str) -> Option<&'c Value> {
        self.table.get(key)
    }

    pub fn maybe_get_int(&self, key: &str) -> Result<Option<i64>> {
        match self.get(key) {
            Some(v) => match v.as_integer() {
                Some(v) => Ok(Some(v)),
                None => Err(Error::InvalidType),
            },
            None => Ok(None),
        }
    }

    pub fn maybe_get_int_typecheck(&self, key: &str) -> crate::Result<Option<i64>> {
        self.maybe_get_int(key)
            .map_err(|_| self.invalid_key(key, "int"))
    }

    pub fn get_str(&self, key: &str) -> Result<&'c str> {
        self.maybe_get_str(key)?.ok_or(Error::MissingKey)
    }

    pub fn maybe_get_str(&self, key: &str) -> Result<Option<&'c str>> {
        match self.get(key) {
            Some(v) => match v.as_str() {
                Some(v) => Ok(Some(v)),
                None => Err(Error::InvalidType),
            },
            None => Ok(None),
        }
    }

    pub fn maybe_get_str_typecheck(&self, key: &str) -> crate::Result<Option<&'c str>> {
        self.maybe_get_str(key)
            .map_err(|_| self.invalid_key(key, "string"))
    }

    pub fn must_get_str(&self, key: &str) -> crate::Result<&'c str> {
        match self.get_str(key) {
            Err(Error::InvalidType) => Err(self.invalid_key(key, "string")),
            Err(Error::MissingKey) => Err(self.missing_key(key)),
            Ok(v) => Ok(v),
        }
    }

    pub fn get_bool(&self, key: &str) -> Result<bool> {
        self.get(key)
            .ok_or(Error::MissingKey)?
            .as_bool()
            .ok_or(Error::InvalidType)
    }

    pub fn get_int(&self, key: &str) -> Result<i64> {
        self.get(key)
            .ok_or(Error::MissingKey)?
            .as_integer()
            .ok_or(Error::InvalidType)
    }

    pub fn get_bool_or(&self, key: &str, default: bool) -> bool {
        self.table
            .get(key)
            .map(|it| it.as_bool().unwrap_or(default))
            .unwrap_or(default)
    }

    pub fn maybe_get_map(&'c self, key: &'c str) -> Result<Option<ConfigMap<'c>>> {
        match self.get(key) {
            Some(v) => match v.as_table() {
                Some(table) => {
                    let name = match &self.get_full_path() {
                        Some(parents) => Cow::Owned(format!("{parents}.{key}")),
                        None => Cow::Borrowed(key),
                    };
                    Ok(Some(Self {
                        name: Some(name),
                        path: self.path,
                        table,
                    }))
                }
                None => Err(Error::InvalidType),
            },
            None => Ok(None),
        }
    }

    pub fn get_map(&'c self, key: &'c str) -> Result<ConfigMap<'c>> {
        self.maybe_get_map(key)?.ok_or(Error::MissingKey)
    }

    pub fn maybe_get_map_typecheck(&'c self, key: &'c str) -> crate::Result<Option<ConfigMap<'c>>> {
        self.maybe_get_map(key)
            .map_err(|_| self.invalid_key(key, "table"))
    }

    pub fn must_get_map(&'c self, key: &'c str) -> crate::Result<ConfigMap<'c>> {
        match self.get_map(key) {
            Err(Error::InvalidType) => Err(self.invalid_key(key, "table")),
            Err(Error::MissingKey) => Err(self.missing_key(key)),
            Ok(v) => Ok(v),
        }
    }
}

#[cfg(test)]
mod test {

    use crate::testing::{global_tmp_context, TestContext};
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use rstest::*;

    #[fixture]
    #[once]
    fn cfg_file(global_tmp_context: &TestContext) -> PathBuf {
        let path = global_tmp_context.get_temp_dir();
        fs::create_dir_all(&path).expect("create tmp dir");
        let file = path.join("config.toml");

        let content = r#"
base = 12

[foo]
bar = "baz"
quux = { neato = true }
"#;

        fs::write(&file, content).expect("failed to write test config");
        file
    }

    #[rstest]
    fn test_config(cfg_file: &PathBuf) {
        let f = |cfg: &ConfigMap| -> crate::Result<()> {
            assert_eq!(cfg.get_int("base").expect("getting base"), 12);
            let foo = cfg.get_map("foo").expect("getting foo");
            assert_eq!(foo.get_str("bar").expect("getting bar"), "baz");
            assert!(foo.get_str("ohno").is_err());
            let quux = foo.get_map("quux").expect("getting foo.quux");
            assert_eq!(quux.get_bool("neato").expect("getting neato"), true);
            assert_eq!(quux.get_bool_or("ohno", false), false);
            assert_eq!(quux.get_bool_or("ohno", true), true);
            Ok(())
        };

        parse_config(cfg_file, f).unwrap();
    }
}
