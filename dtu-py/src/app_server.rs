use std::{borrow::Cow, collections::HashMap};

use dtu::{
    app_server::{
        extract_string_from_json, maybe_extract_string_from_json, AppServer, CallAppService,
        CallSystemService, Command, CommandResult, IntentData, ProviderCommand, ProviderSubcommand,
        TcpAppServer,
    },
    utils::ClassName,
};

use pyo3::prelude::*;

use crate::{
    context::PyContext,
    exception::DtuError,
    intent_string::build_intent_string,
    parcel_string::{build_parcel_string, ParcelValue},
};

#[pyclass(name = "AppServer")]
pub struct PyAppServer(TcpAppServer);

struct AppServerError(dtu::app_server::Error);

impl From<dtu::app_server::Error> for AppServerError {
    fn from(value: dtu::app_server::Error) -> Self {
        Self(value)
    }
}

impl From<AppServerError> for PyErr {
    fn from(value: AppServerError) -> Self {
        DtuError::new_err(value.0.to_string())
    }
}

type Result<T> = std::result::Result<T, AppServerError>;

/// Allows interaction with the application server running on the device. This requires both the dtu
/// test application installed and the port forwarded appropriately to work.
#[pymethods]
impl PyAppServer {
    /// Create a new AppServer from the given Context. This is the recommended way to create a new
    /// application server connection, as it will respect settings that change the port.
    #[new]
    #[pyo3(signature = (ctx = None))]
    fn new(ctx: Option<&PyContext>) -> PyResult<Self> {
        Ok(Self(
            match ctx {
                Some(v) => TcpAppServer::from_ctx(v),
                None => TcpAppServer::from_ctx(&dtu::DefaultContext::new()),
            }
            .map_err(|e| DtuError::new_err(e.to_string()))?,
        ))
    }

    /// Connect to the application server at the given address and port.
    #[staticmethod]
    fn connect(addr: &str, port: u16) -> PyResult<Self> {
        Ok(Self(
            TcpAppServer::connect(addr, port).map_err(|e| DtuError::new_err(e.to_string()))?,
        ))
    }

    /// Run a shell command in the context of the test application
    ///
    /// The optional `shell` parameter allows choosing the shell program to use, otherwise the
    /// default `/system/bin/sh` is used.
    #[pyo3(signature = (cmd, *, shell = None))]
    fn sh(&mut self, cmd: &str, shell: Option<&str>) -> Result<PyCommandResult> {
        Ok(PyCommandResult::from(self.0.sh_with_shell(cmd, shell)?))
    }

    #[pyo3(signature = (uri, *, method, arg = None))]
    fn provider_call(&mut self, uri: &str, method: &str, arg: Option<&str>) -> Result<String> {
        Ok(self.0.provider_call(uri, method, arg)?)
    }

    fn provider_insert(
        &mut self,
        uri: &str,
        data: HashMap<String, ParcelValue>,
    ) -> Result<Option<String>> {
        let raw_data = build_intent_string(&data);
        let cmd = ProviderSubcommand::Insert { data: &raw_data };
        let cmd = ProviderCommand { uri, cmd };
        let res = self.0.send_command(Command::Provider, &cmd)?;
        Ok(maybe_extract_string_from_json("uri", &res)?)
    }

    /// Query the given provider
    ///
    /// The `query_args` must be provided as `ParcelValue` instances
    #[pyo3(signature = (uri, *, projection = None, selection = None, selection_args = None, query_args = None, sort_order = None))]
    fn provider_query(
        &mut self,
        uri: &str,
        projection: Option<Vec<String>>,
        selection: Option<&str>,
        selection_args: Option<Vec<String>>,
        query_args: Option<Vec<ParcelValue>>,
        sort_order: Option<&str>,
    ) -> Result<String> {
        let qa = query_args.as_ref().map(|e| build_parcel_string(e));

        let cmd = ProviderSubcommand::Query {
            projection: projection.as_ref().map(Vec::as_slice),
            selection,
            selection_args: selection_args.as_ref().map(Vec::as_slice),
            query_args: qa.as_ref().map(String::as_str),
            sort_order,
        };

        let cmd = ProviderCommand { uri, cmd };
        Ok(self.0.send_command(Command::Provider, &cmd)?)
    }

    #[pyo3(signature = (uri, *, where_clause = None, selection_args = None))]
    fn provider_delete(
        &mut self,
        uri: &str,
        where_clause: Option<&str>,
        selection_args: Option<Vec<String>>,
    ) -> Result<i64> {
        Ok(self
            .0
            .provider_delete(uri, where_clause, selection_args.as_deref())?)
    }

    fn provider_read(&mut self, uri: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.0.provider_read(uri)?.data_raw())
    }

    fn provider_write(&mut self, uri: &str, data: Vec<u8>) -> Result<String> {
        Ok(self.0.provider_write(uri, &data)?)
    }

    fn system_service_shell_cmd(
        &mut self,
        service: &str,
        cmd: Option<&str>,
        timeout: u32,
    ) -> Result<PyCommandResult> {
        Ok(PyCommandResult::from(
            self.0.system_service_shell_cmd(service, cmd, timeout)?,
        ))
    }

    /// Call a method on the given system service.
    ///
    /// The parcel_data must be provided as a list of `ParcelValue` instances
    #[pyo3(signature = (name, txn, *, iface = None, parcel_data = None))]
    fn call_system_service(
        &mut self,
        name: &str,
        txn: u32,
        iface: Option<&str>,
        parcel_data: Option<Vec<ParcelValue>>,
    ) -> Result<String> {
        let iface = iface.map(ClassName::from);
        let parcel_data = parcel_data.as_ref().map(build_parcel_string);
        let payload = CallSystemService {
            name,
            txn,
            iface: iface.as_ref(),
            parcel_data: parcel_data.as_ref().map(String::as_str),
        };
        let res = self.0.send_command(Command::SystemService, &payload)?;
        Ok(extract_string_from_json("response", &res)?)
    }

    /// Call a method on the given application service.
    ///
    /// The parcel_data must be provided as a list of `ParcelValue` instances
    #[pyo3(signature = (txn, package, class, *, iface = None, action = None, parcel_data = None))]
    fn call_app_service(
        &mut self,
        txn: u32,
        package: &str,
        class: &str,
        iface: Option<&str>,
        action: Option<&str>,
        parcel_data: Option<Vec<ParcelValue>>,
    ) -> Result<String> {
        let class = ClassName::from(class);
        let iface = iface.map(ClassName::from);
        let parcel_data = parcel_data.as_ref().map(build_parcel_string);
        let payload = CallAppService {
            txn,
            package,
            class: class.as_ref(),
            action,
            iface: iface.as_ref(),
            parcel_data: parcel_data.as_ref().map(String::as_str),
        };
        let res = self.0.send_command(Command::AppService, &payload)?;
        Ok(extract_string_from_json("response", &res)?)
    }

    /// Send a broadcast from the context of the test application
    ///
    /// The `intent_data` values must be provided as a map of strings to `ParcelValue` instances
    #[pyo3(signature = (action = None, data = None, package = None, class = None, flags = None, intent_data = None))]
    fn broadcast(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<Vec<String>>,
        intent_data: Option<HashMap<String, ParcelValue>>,
    ) -> Result<String> {
        let intent_data = intent_data.as_ref().map(build_intent_string);
        let payload = IntentData {
            action,
            data,
            package,
            class,
            flags: flags.as_ref(),
            intent_data: intent_data.as_ref().map(String::as_str),
        };

        Ok(self.0.send_command(Command::Broadcast, &payload)?)
    }

    /// Start a service from the context of the test application
    ///
    /// The `intent_data` values must be provided as a map of strings to `ParcelValue` instances
    #[pyo3(signature = (action = None, data = None, package = None, class = None, flags = None, intent_data = None))]
    fn start_service(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<Vec<String>>,
        intent_data: Option<HashMap<String, ParcelValue>>,
    ) -> Result<String> {
        let intent_data = intent_data.as_ref().map(build_intent_string);
        let payload = IntentData {
            action,
            data,
            package,
            class,
            flags: flags.as_ref(),
            intent_data: intent_data.as_ref().map(String::as_str),
        };

        Ok(self.0.send_command(Command::StartService, &payload)?)
    }

    /// Start an activity from the context of the test application
    ///
    /// The `intent_data` values must be provided as a map of strings to `ParcelValue` instances
    #[pyo3(signature = (action = None, data = None, package = None, class = None, flags = None, intent_data = None))]
    fn start_activity(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<Vec<String>>,
        intent_data: Option<HashMap<String, ParcelValue>>,
    ) -> Result<String> {
        let intent_data = intent_data.as_ref().map(build_intent_string);
        let payload = IntentData {
            action,
            data,
            package,
            class,
            flags: flags.as_ref(),
            intent_data: intent_data.as_ref().map(String::as_str),
        };

        Ok(self.0.send_command(Command::StartActivity, &payload)?)
    }
}

#[pyclass(frozen, name = "CommandResult")]
#[derive(Clone)]
pub struct PyCommandResult(pub(crate) CommandResult);

impl From<CommandResult> for PyCommandResult {
    fn from(v: CommandResult) -> Self {
        Self(v)
    }
}

#[pymethods]
impl PyCommandResult {
    #[getter]
    fn exit(&self) -> i32 {
        self.0.exit
    }

    #[getter]
    fn stdout(&self) -> &[u8] {
        &self.0.stdout
    }

    #[getter]
    fn stderr(&self) -> &[u8] {
        &self.0.stderr
    }

    fn ok(&self) -> bool {
        self.0.ok()
    }

    fn stdout_string(&self) -> Cow<'_, str> {
        self.0.stdout_string()
    }

    fn stderr_string(&self) -> Cow<'_, str> {
        self.0.stderr_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "CommandResult(exit={}, stdout_len={}, stderr_len={})",
            self.0.exit,
            self.0.stdout.len(),
            self.0.stderr.len()
        )
    }
}
