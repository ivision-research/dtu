use dtu::filestore::{get_filestore, FileStore};
use pyo3::prelude::*;

use crate::{context::PyContext, exception::DtuBaseError};

#[pyclass(name = "FileStore")]
pub struct PyFileStore {
    ctx: PyContext,
    fs: Box<dyn FileStore>,
}

type Result<T> = std::result::Result<T, DtuBaseError>;

#[pymethods]
impl PyFileStore {
    #[new]
    #[pyo3(signature = (ctx = None))]
    fn new(ctx: Option<PyContext>) -> Result<Self> {
        let ctx = ctx.unwrap_or_else(PyContext::default);
        let fs = get_filestore(&ctx)?;
        Ok(PyFileStore { ctx, fs })
    }

    /// Put the file at `local_path` into the store at `remote_path`
    fn put_file(&self, local_path: &str, remote_path: &str) -> Result<()> {
        Ok(self.fs.put_file(&self.ctx, local_path, remote_path)?)
    }

    /// Retrive the file `remote_path` from the store and write it to `local_path`
    fn get_file(&self, remote_path: &str, local_path: &str) -> Result<()> {
        Ok(self.fs.get_file(&self.ctx, remote_path, local_path)?)
    }

    /// List files in the given directory.
    #[pyo3(signature = (dir = None))]
    fn list_files(&self, dir: Option<&str>) -> Result<Vec<String>> {
        Ok(self.fs.list_files(&self.ctx, dir)?)
    }

    /// Remove the given file
    fn remove_file(&self, file: &str) -> Result<()> {
        Ok(self.fs.remove_file(&self.ctx, file)?)
    }

    fn name(&self) -> &'static str {
        self.fs.name()
    }
}
