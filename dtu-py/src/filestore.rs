use dtu::filestore::{get_filestore, FileStore};
use pyo3::prelude::*;

use crate::{context::PyContext, exception::DtuBaseError};

#[pyclass(name = "FileStore")]
pub struct PyFileStore(Box<dyn FileStore>);

type Result<T> = std::result::Result<T, DtuBaseError>;

#[pymethods]
impl PyFileStore {
    #[new]
    fn new(ctx: &PyContext) -> Result<Self> {
        Ok(PyFileStore(get_filestore(ctx)?))
    }

    /// Put the file at `local_path` into the store at `remote_path`
    fn put_file(&self, ctx: &PyContext, local_path: &str, remote_path: &str) -> Result<()> {
        Ok(self.0.put_file(ctx, local_path, remote_path)?)
    }

    /// Retrive the file `remote_path` from the store and write it to `local_path`
    fn get_file(&self, ctx: &PyContext, remote_path: &str, local_path: &str) -> Result<()> {
        Ok(self.0.get_file(ctx, remote_path, local_path)?)
    }

    /// List files in the given directory.
    #[pyo3(signature = (ctx, dir = None))]
    fn list_files(&self, ctx: &PyContext, dir: Option<&str>) -> Result<Vec<String>> {
        Ok(self.0.list_files(ctx, dir)?)
    }

    /// Remove the given file
    fn remove_file(&self, ctx: &PyContext, file: &str) -> Result<()> {
        Ok(self.0.remove_file(ctx, file)?)
    }

    fn name(&self) -> &'static str {
        self.0.name()
    }
}
