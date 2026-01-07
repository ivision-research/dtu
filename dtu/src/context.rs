use blanket::blanket;
use once_cell::sync::OnceCell;
use std::env;
use std::path::PathBuf;

use directories::BaseDirs;
use which::{which, which_in};

use crate::adb::{Adb, ExecAdb};
use crate::config::Config;
use crate::utils::ensure_dir_exists;
use crate::Error;

use crossbeam::atomic::AtomicCell;
use log;

use std::ops::DerefMut;
use std::sync::Mutex;

#[derive(Clone)]
struct CachedBin {
    name: String,
    path: String,
}

fn wrapped_which(bin: &str) -> Option<PathBuf> {
    if let Ok(dtu_path) = env::var("DTU_PATH") {
        let cwd = env::current_dir().ok()?;
        return which_in(bin, Some(&dtu_path), &cwd).ok();
    }
    which(bin).ok()
}

fn which_find_program(bin: &str) -> Option<String> {
    wrapped_which(bin).map(|it| it.to_string_lossy().into())
}

#[inline(always)]
fn find_program(prog: &str) -> Option<String> {
    which_find_program(prog)
}

/// Context is a trait for an object that can help standardize file locations,
/// find binaries, and lookup env vars.
///
/// Most methods on this trait have a default implementation that is perfectly
/// safe to leave unchanged.
#[blanket(derive(Ref, Box))]
pub trait Context: Send + Sync {
    /// Returns the target Android API level.
    fn get_target_api_level(&self) -> u32;

    fn maybe_get_env(&self, key: &str) -> Option<String> {
        env::var(key).ok()
    }

    fn maybe_get_bin(&self, bin: &str) -> Option<String> {
        find_program(bin)
    }

    fn has_bin(&self, bin: &str) -> bool {
        self.maybe_get_bin(bin).is_some()
    }

    fn get_bin(&self, bin: &str) -> crate::Result<String> {
        self.maybe_get_bin(bin)
            .ok_or_else(|| Error::MissingBin(bin.into()))
    }

    fn unchecked_get_bin(&self, bin: &str) -> String {
        self.maybe_get_bin(bin)
            .expect(&format!("unchecked_get_bin({})", bin))
    }

    fn has_env(&self, key: &str) -> bool {
        self.maybe_get_env(key).is_some()
    }
    fn get_env(&self, key: &str) -> crate::Result<String> {
        self.maybe_get_env(key)
            .ok_or_else(|| Error::MissingEnv(key.into()))
    }

    fn unchecked_get_env(&self, key: &str) -> String {
        self.maybe_get_env(key)
            .expect(&format!("unchecked_get_env({})", key))
    }

    fn get_project_dir(&self) -> crate::Result<PathBuf> {
        let home = self
            .get_env("DTU_PROJECT_HOME")
            .map(|env| PathBuf::new().join(env))?;
        if !home.exists() {
            return Err(Error::Generic(format!(
                "DTU_PROJECT_HOME set to {}, but that directory doesn't exist",
                home.to_str().expect("valid paths")
            )));
        }
        Ok(home)
    }

    fn get_project_config_file(&self) -> crate::Result<PathBuf> {
        self.get_project_dir_child("dtu.toml")
    }

    fn get_project_config<'a>(&'a self) -> crate::Result<Option<&'a Config>>;

    fn get_test_app_dir(&self) -> crate::Result<PathBuf> {
        self.get_project_dir_child("test_app")
    }

    fn get_project_dir_child(&self, child: &str) -> crate::Result<PathBuf> {
        self.get_project_dir().map(|x| x.join(child))
    }

    fn get_output_dir(&self) -> crate::Result<PathBuf> {
        self.get_project_dir_child("dtu_out")
    }

    fn get_output_dir_child(&self, child: &str) -> crate::Result<PathBuf> {
        self.get_output_dir().map(|x| x.join(child))
    }

    /// Get a cache dir relative to the project instead of the user's cache dir
    fn get_project_cache_dir(&self) -> crate::Result<PathBuf> {
        let cache = self.get_output_dir_child("cache")?;
        ensure_dir_exists(&cache)?;
        Ok(cache)
    }

    fn get_cache_dir(&self) -> crate::Result<PathBuf> {
        let dir =
            BaseDirs::new().ok_or_else(|| Error::Generic(format!("failed to get BaseDirs")))?;
        let cache = dir.cache_dir().to_path_buf().join("dtu");
        ensure_dir_exists(&cache)?;
        Ok(cache)
    }

    fn get_smalisa_analysis_dir(&self) -> crate::Result<PathBuf> {
        self.get_output_dir_child("smalisa-output")
    }

    fn get_graph_import_dir(&self) -> crate::Result<PathBuf> {
        self.get_smalisa_analysis_dir()
    }

    fn get_selinux_dir(&self) -> crate::Result<PathBuf> {
        self.get_output_dir_child("selinux")
    }

    fn get_sqlite_dir(&self) -> crate::Result<PathBuf> {
        self.get_output_dir_child("sqlite")
    }

    fn get_frameworks_dir(&self) -> crate::Result<PathBuf> {
        self.get_output_dir_child("framework_files")
    }

    fn get_apks_dir(&self) -> crate::Result<PathBuf> {
        self.get_output_dir_child("apks")
    }

    fn get_smali_dir(&self) -> crate::Result<PathBuf> {
        self.get_output_dir_child("smali")
    }

    fn get_user_local_dir(&self) -> crate::Result<PathBuf> {
        let bd = BaseDirs::new().ok_or(Error::NoBaseDirs)?;
        Ok(bd.data_local_dir().join("dtu"))
    }

    fn get_user_config_dir(&self) -> crate::Result<PathBuf> {
        let bd = BaseDirs::new().ok_or(Error::NoBaseDirs)?;
        Ok(bd.config_dir().join("dtu"))
    }
}

pub struct DefaultContext {
    target_api_level: AtomicCell<Option<u32>>,
    bin_cache: Mutex<Vec<CachedBin>>,
    project_config: OnceCell<Option<Config>>,
}

impl Clone for DefaultContext {
    fn clone(&self) -> Self {
        let target_api_level = self.target_api_level.load();
        let cache = self.bin_cache.lock().expect("failed to lock");
        let project_config = self.project_config.clone();
        Self {
            target_api_level: AtomicCell::new(target_api_level),
            bin_cache: Mutex::new(cache.clone()),
            project_config,
        }
    }
}

impl DefaultContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_target_api_level(&mut self, target_api_level: u32) -> &mut Self {
        self.target_api_level = AtomicCell::new(Some(target_api_level));
        self
    }
}

impl Default for DefaultContext {
    fn default() -> Self {
        let project_config = OnceCell::new();
        Self {
            target_api_level: AtomicCell::new(None),
            bin_cache: Mutex::new(Vec::new()),
            project_config,
        }
    }
}

impl Context for DefaultContext {
    fn get_project_config<'a>(&'a self) -> crate::Result<Option<&'a Config>> {
        let cfg = self
            .project_config
            .get_or_try_init(|| -> crate::Result<Option<Config>> {
                let path = self.get_project_config_file()?;
                if !path.exists() {
                    Ok(None)
                } else {
                    Ok(Some(Config::parse(&path)?))
                }
            })?;
        Ok(cfg.as_ref())
    }

    fn get_target_api_level(&self) -> u32 {
        if let Some(lvl) = self.target_api_level.load() {
            return lvl;
        }
        // Allow it to be set via env var
        if let Some(lvl) = self.get_api_level_via_env() {
            self.target_api_level.store(Some(lvl));
            return lvl;
        }

        if let Some(lvl) = self.get_api_level_via_adb() {
            self.target_api_level.store(Some(lvl));
            return lvl;
        }

        // Oh well, hard code it
        log::warn!("failed to determine api level, use hard coded value {}", 33);
        self.target_api_level.store(Some(33));
        33
    }

    fn maybe_get_env(&self, key: &str) -> Option<String> {
        env::var(key).ok()
    }

    fn maybe_get_bin(&self, prog: &str) -> Option<String> {
        let mut cache_guard = self.bin_cache.lock().expect("failed to lock");
        let cache = cache_guard.deref_mut();
        let mut it = cache.iter();
        while let Some(val) = it.next() {
            if val.name == prog {
                return Some(val.path.clone());
            }
        }

        let found = find_program(prog)?;

        cache.push(CachedBin {
            name: prog.into(),
            path: found.clone(),
        });

        Some(found)
    }
}

impl DefaultContext {
    fn get_api_level_via_env(&self) -> Option<u32> {
        if let Some(env_level) = self.maybe_get_env("DTU_ANDROID_API_LEVEL") {
            if let Ok(lvl) = u32::from_str_radix(&env_level, 10) {
                return Some(lvl);
            }
        }
        None
    }

    fn get_api_level_via_adb(&self) -> Option<u32> {
        let adb = ExecAdb::new(self).ok()?;

        let mut lvl = match adb.shell("getprop ro.build.version.sdk") {
            Ok(v) => match v.err_on_status() {
                Ok(v) => str::parse::<u32>(v.stdout_utf8_lossy().trim()).ok(),
                Err(_) => None,
            },
            Err(_) => None,
        };

        if lvl.is_none() {
            let res = adb
                .shell("getprop | grep '\\[ro.build.version.sdk\\]")
                .ok()?;

            if !res.ok() {
                return None;
            }
            let stdout = res.stdout_utf8_lossy();
            lvl = adb_version_prop_line_to_level(&stdout);
        }

        lvl
    }
}

fn adb_version_prop_line_to_level(line: &str) -> Option<u32> {
    let idx = line.rfind('[')?;
    let chars = line.chars();
    let mut level = 0u32;
    for c in chars.skip(idx) {
        if c == ']' || c == '.' {
            break;
        }
        if let Some(digit) = c.to_digit(10) {
            level = (level * 10) + digit;
        }
    }

    if level > 0 {
        Some(level)
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_version_prop_line_to_level() {
        let line = "[ro.build.version.sdk]: [31]";

        assert_eq!(adb_version_prop_line_to_level(line), Some(31));
    }
}
