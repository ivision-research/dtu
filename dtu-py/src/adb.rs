use dtu::adb::{Adb, ExecAdb};

use pyo3::prelude::*;

use crate::{context::PyContext, exception::DtuBaseError, types::PyCmdOutput};

#[pyclass(name = "Adb")]
pub struct PyAdb(ExecAdb);

type Result<T> = std::result::Result<T, DtuBaseError>;

#[pymethods]
impl PyAdb {
    #[new]
    fn new(ctx: &PyContext) -> Result<Self> {
        Ok(Self(ExecAdb::new(ctx)?))
    }

    fn install(&self, apk: &str) -> Result<()> {
        Ok(self.0.install(apk)?)
    }

    fn uninstall(&self, apk: &str) -> Result<()> {
        Ok(self.0.uninstall(apk)?)
    }

    fn pull(&self, device: &str, local: &str) -> Result<PyCmdOutput> {
        Ok(self.0.pull(device, local)?.into())
    }

    fn push(&self, local: &str, device: &str) -> Result<PyCmdOutput> {
        Ok(self.0.push(local, device)?.into())
    }

    fn shell(&self, shell_cmd: &str) -> Result<PyCmdOutput> {
        Ok(self.0.shell(shell_cmd)?.into())
    }

    fn reverse_tcp_port(&self, local_port: u16, remote_port: u16) -> Result<PyCmdOutput> {
        Ok(self.0.reverse_tcp_port(local_port, remote_port)?.into())
    }

    fn forward_tcp_port(&self, local_port: u16, remote_port: u16) -> Result<PyCmdOutput> {
        Ok(self.0.forward_tcp_port(local_port, remote_port)?.into())
    }

    fn forward_generic(&self, local: &str, remote: &str) -> Result<PyCmdOutput> {
        Ok(self.0.forward_generic(local, remote)?.into())
    }

    fn reverse_generic(&self, local: &str, remote: &str) -> Result<PyCmdOutput> {
        Ok(self.0.reverse_generic(local, remote)?.into())
    }
}
