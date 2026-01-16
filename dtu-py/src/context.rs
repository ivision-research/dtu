use std::path::PathBuf;

use dtu::{config::Config, Context, DefaultContext};
use pyo3::prelude::*;

use crate::exception::DtuBaseError;

#[derive(Clone)]
#[pyclass(module = "dtu", name = "Context")]
pub struct PyContext(DefaultContext);

type Result<T> = std::result::Result<T, DtuBaseError>;

impl Default for PyContext {
    fn default() -> Self {
        Self(DefaultContext::new())
    }
}

#[pymethods]
impl PyContext {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn get_target_api_level(&self) -> u32 {
        self.0.get_target_api_level()
    }

    fn maybe_get_env(&self, key: &str) -> Option<String> {
        self.0.maybe_get_env(key)
    }

    fn maybe_get_bin(&self, bin: &str) -> Option<String> {
        self.0.maybe_get_bin(bin)
    }

    fn has_bin(&self, bin: &str) -> bool {
        self.0.has_bin(bin)
    }

    fn get_bin(&self, bin: &str) -> Result<String> {
        Ok(self.0.get_bin(bin)?)
    }

    fn unchecked_get_bin(&self, bin: &str) -> String {
        self.0.unchecked_get_bin(bin)
    }

    fn has_env(&self, key: &str) -> bool {
        self.0.has_env(key)
    }

    fn get_env(&self, key: &str) -> Result<String> {
        Ok(self.0.get_env(key)?)
    }

    fn unchecked_get_env(&self, key: &str) -> String {
        self.0.unchecked_get_env(key)
    }

    fn get_project_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_project_dir()?)
    }

    fn get_project_config_file(&self) -> Result<PathBuf> {
        Ok(self.0.get_project_config_file()?)
    }

    fn get_test_app_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_test_app_dir()?)
    }

    fn get_project_dir_child(&self, child: &str) -> Result<PathBuf> {
        Ok(self.0.get_project_dir_child(child)?)
    }

    fn get_output_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_output_dir()?)
    }

    fn get_output_dir_child(&self, child: &str) -> Result<PathBuf> {
        Ok(self.0.get_output_dir_child(child)?)
    }

    fn get_project_cache_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_project_cache_dir()?)
    }

    fn get_cache_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_cache_dir()?)
    }

    fn get_smalisa_analysis_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_smalisa_analysis_dir()?)
    }

    fn get_graph_import_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_graph_import_dir()?)
    }

    fn get_selinux_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_selinux_dir()?)
    }

    fn get_sqlite_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_sqlite_dir()?)
    }

    fn get_frameworks_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_frameworks_dir()?)
    }

    fn get_apks_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_apks_dir()?)
    }

    fn get_smali_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_smali_dir()?)
    }

    fn get_user_local_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_user_local_dir()?)
    }

    fn get_user_config_dir(&self) -> Result<PathBuf> {
        Ok(self.0.get_user_config_dir()?)
    }
}

impl dtu::Context for PyContext {
    fn get_target_api_level(&self) -> u32 {
        self.0.get_target_api_level()
    }

    fn get_project_config<'a>(&'a self) -> dtu::Result<Option<&'a Config>> {
        self.0.get_project_config()
    }
}
