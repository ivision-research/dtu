#![allow(unused)]
use std::collections::HashMap;
use std::fs::{create_dir_all, OpenOptions};
use std::path::{Path, PathBuf};
use std::{env, fs};
use which::which;

use anyhow::Context as AnyhowContext;
use mockall::mock;
use rand::Rng;
use rstest::fixture;

use crate::utils::{ensure_dir_exists, path_must_str};
use crate::Context;

use crate::config::Config;

#[fixture]
pub fn tmp_context() -> TestContext {
    TestContext::default()
}

#[fixture]
pub fn mock_context() -> MockContext {
    MockContext::new()
}

#[fixture]
#[once]
pub fn global_tmp_context() -> TestContext {
    TestContext::default()
}

pub struct TestContext {
    base_dir: PathBuf,
    home_dir: PathBuf,
    env: HashMap<String, String>,
    bins: HashMap<String, String>,
    api_level: u32,
}

pub enum TreeEntry<'a> {
    Dir,
    EmptyFile,
    TxtFile(&'a str),
    BinFile(&'a [u8]),
}

impl TestContext {
    pub const API_LEVEL: u32 = 33;

    pub fn set_env<K: AsRef<str>, V: AsRef<str>>(&mut self, key: K, value: V) -> &mut Self {
        self.env.insert(key.as_ref().into(), value.as_ref().into());
        self
    }

    pub fn set_bin<K: AsRef<str>, V: AsRef<str>>(&mut self, key: K, bin: V) -> &mut Self {
        self.bins.insert(key.as_ref().into(), bin.as_ref().into());
        self
    }

    /// Create a collection of files with the given names and contexts
    ///
    /// The tree is rooted at the base directory
    pub fn create_tree(&self, tree: &[(&str, TreeEntry)]) -> anyhow::Result<()> {
        for (relative, content) in tree {
            let file = self.base_dir.join(relative);
            if let Some(parent) = file.parent() {
                if !parent.exists() {
                    create_dir_all(&parent)
                        .with_context(|| format!("creating parent dirs for {relative}"))?;
                }
            }

            match content {
                TreeEntry::Dir => {
                    fs::create_dir(&file).with_context(|| format!("creating dir {relative}"))?
                }
                TreeEntry::EmptyFile => {
                    OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&file)
                        .with_context(|| format!("creating {relative}"))?;
                }
                TreeEntry::BinFile(content) => fs::write(&file, content)
                    .with_context(|| format!("writing content to {relative}"))?,
                TreeEntry::TxtFile(content) => fs::write(&file, content)
                    .with_context(|| format!("writing content to {relative}"))?,
            }
        }
        Ok(())
    }

    pub fn to_abs<P: AsRef<Path> + ?Sized>(&self, path: &P) -> PathBuf {
        self.base_dir.join(path)
    }

    pub fn to_abs_string<P: AsRef<Path> + ?Sized>(&self, path: &P) -> String {
        path_must_str(&self.base_dir.join(path)).into()
    }

    pub fn get_base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn get_temp_path(&self, suffix: Option<&str>) -> PathBuf {
        let mut rng = rand::thread_rng();
        let rand_name: u64 = rng.gen();
        let name = match suffix {
            Some(v) => format!("{}.{}", rand_name, v),
            None => rand_name.to_string(),
        };
        self.base_dir.join(name)
    }

    pub fn get_temp_dir(&self) -> PathBuf {
        self.get_temp_path(None)
    }

    pub fn new_tmp_file(&self, content: &str) -> anyhow::Result<PathBuf> {
        self.new_tmp_file_suffix(None, content)
    }

    pub fn new_tmp_file_suffix(
        &self,
        suffix: Option<&str>,
        content: &str,
    ) -> anyhow::Result<PathBuf> {
        let path = self.get_temp_path(suffix);
        fs::write(&path, content).with_context(|| "failed to write content to temp file")?;
        Ok(path)
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        let td = &self.base_dir;
        if td.exists() {
            fs::remove_dir_all(td).expect("failed to clear test dir");
        }
    }
}

impl Default for TestContext {
    fn default() -> Self {
        let mut rng = rand::thread_rng();
        let rand_name: u64 = rng.gen();
        let td = env::temp_dir().join(format!("dtu_test_base_{}", rand_name));

        if td.exists() {
            fs::remove_dir_all(&td).expect("failed to clear test dir");
        }

        let home_dir = td.join("project_home");

        ensure_dir_exists(&home_dir).expect("failed to create default test dir");

        let mut env = HashMap::new();
        env.insert("DTU_PROJECT_HOME".into(), td.to_string_lossy().into());
        env.insert("ANDROID_SERIAL".into(), "TESTSERIAL".into());

        let mut bins = HashMap::new();

        let mut it = Self {
            base_dir: td,
            home_dir,
            api_level: Self::API_LEVEL,
            env,
            bins,
        };

        // If `dtu-test-adb` is installed set it to `adb` for testing purposes
        match which("dtu-test-adb") {
            Ok(v) => {
                it.bins.insert("adb".into(), path_must_str(&v).into());
            }
            Err(_) => {}
        }

        it
    }
}

impl Context for TestContext {
    fn get_target_api_level(&self) -> u32 {
        self.api_level
    }

    fn maybe_get_env(&self, key: &str) -> Option<String> {
        self.env.get(key).map(String::from)
    }

    fn maybe_get_bin(&self, bin: &str) -> Option<String> {
        self.bins.get(bin).map(String::from)
    }

    fn get_cache_dir(&self) -> crate::Result<PathBuf> {
        Ok(self.base_dir.join("cache"))
    }

    fn get_user_config_dir(&self) -> crate::Result<PathBuf> {
        Ok(self.base_dir.join("config"))
    }

    fn get_user_local_dir(&self) -> crate::Result<PathBuf> {
        Ok(self.base_dir.join("local"))
    }

    fn get_project_config<'a>(&'a self) -> crate::Result<Option<&'a Config>> {
        Ok(None)
    }
}

mock! {
    pub Context {

    }

    impl crate::Context for Context {
        fn get_target_api_level(&self) -> u32;
        fn maybe_get_env(&self, key: &str) -> Option<String>;
        fn maybe_get_bin(&self, bin: &str) -> Option<String>;
        fn get_project_dir(&self) -> crate::Result<PathBuf>;
        fn get_project_config<'a>(&'a self) -> crate::Result<Option<&'a Config>>;
    }
}
