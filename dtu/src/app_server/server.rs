use crate::utils::{unbase64, ClassName};
use serde::de::{DeserializeOwned, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::de::StrRead;
use serde_json::Value;
use std::borrow::Cow;
use std::io;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};

use super::{IntentString, ParcelString};
use crate::command::split;
use crate::Context;

use crate::utils::HEX_BYTES;

pub trait AppServer {
    fn sh(&mut self, cmd: &str) -> Result<CommandResult> {
        self.sh_with_shell(cmd, None)
    }
    fn sh_with_shell(&mut self, cmd: &str, shell: Option<&str>) -> Result<CommandResult>;

    fn provider_call(&mut self, uri: &str, method: &str, arg: Option<&str>) -> Result<String>;

    fn provider_query(
        &mut self,
        uri: &str,
        projection: Option<&[String]>,
        selection: Option<&str>,
        selection_args: Option<&[String]>,
        query_args: Option<&ParcelString>,
        sort_order: Option<&str>,
    ) -> Result<String>;

    fn provider_insert(&mut self, uri: &str, data: &IntentString) -> Result<Option<String>>;
    fn provider_delete(
        &mut self,
        uri: &str,
        where_clause: Option<&str>,
        selection_args: Option<&[String]>,
    ) -> Result<i64>;

    fn provider_read(&mut self, uri: &str) -> Result<ProviderReadContent>;
    fn provider_write(&mut self, uri: &str, data: &[u8]) -> Result<String>;

    fn call_system_service(
        &mut self,
        name: &str,
        txn: u32,
        iface: Option<&ClassName>,
        parcel_data: Option<&ParcelString>,
    ) -> Result<String>;

    fn call_app_service(
        &mut self,
        txn: u32,
        package: &str,
        class: &ClassName,
        iface: Option<&ClassName>,
        action: Option<&str>,
        parcel_data: Option<&ParcelString>,
    ) -> Result<String>;

    fn broadcast(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<&Vec<String>>,
        intent_data: Option<&IntentString>,
    ) -> Result<String>;

    fn start_activity(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<&Vec<String>>,
        intent_data: Option<&IntentString>,
    ) -> Result<String>;

    fn start_service(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<&Vec<String>>,
        intent_data: Option<&IntentString>,
    ) -> Result<String>;

    fn run_test(
        &mut self,
        test_name: &str,
        intent_data: Option<&IntentString>,
    ) -> Result<TestOutput>;

    fn system_service_shell_cmd(
        &mut self,
        service: &str,
        cmd: Option<&str>,
        timeout: u32,
    ) -> Result<CommandResult>;
}

#[derive(Deserialize)]
pub struct TestOutput {
    pub success: bool,
    pub output: String,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Deserialize, Clone)]
pub struct CommandResult {
    pub exit: i32,
    #[serde(deserialize_with = "deserialize_b64")]
    pub stdout: Vec<u8>,
    #[serde(deserialize_with = "deserialize_b64")]
    pub stderr: Vec<u8>,
}

impl CommandResult {
    pub fn ok(&self) -> bool {
        self.exit == 0
    }

    pub fn stdout_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.stdout.as_slice())
    }

    pub fn stderr_string(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.stderr.as_slice())
    }
}

pub struct TcpAppServer {
    stream: TcpStream,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("server error: {0}")]
    ServerError(String),

    #[error("io error: {0}")]
    IO(io::Error),

    #[error("the server returned an invalid response")]
    InvalidResponse,

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::IO(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidInput(value.to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("invalid address {0}")]
    InvalidAddress(String),

    #[error("failed to connect to {addr}:{port}: {err}")]
    ConnectFailed {
        addr: String,
        port: u16,
        err: io::Error,
    },

    #[error("invalid app server port from environment")]
    InvalidEnvPort,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
struct SystemServiceShellCommand<'a> {
    service: &'a str,
    command: Vec<String>,
    timeout: u32,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
struct ShellCommand<'a> {
    cmd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<&'a str>,
}

#[derive(Default)]
pub struct ProviderUriBuilder<'a> {
    authority: &'a str,
    path: Option<&'a str>,
    query: Option<&'a str>,
}

impl<'a> ProviderUriBuilder<'a> {
    pub fn new(authority: &'a str) -> Self {
        Self {
            authority,
            path: None,
            query: None,
        }
    }

    pub fn with_query(&mut self, query: &'a str) -> &mut Self {
        self.query = Some(query);
        self
    }

    pub fn with_path(&mut self, path: &'a str) -> &mut Self {
        self.path = Some(path);
        self
    }

    pub fn build(&self) -> String {
        let mut s = format!("content://{}", self.authority);
        if let Some(p) = self.path {
            if !p.starts_with('/') {
                s.push('/');
            }
            s.push_str(p);
        }

        if let Some(q) = self.query {
            if !q.starts_with('?') {
                s.push('?');
            }
            s.push_str(q);
        }
        s
    }
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
pub struct ProviderCommand<'a> {
    pub uri: &'a str,

    #[serde(flatten)]
    pub cmd: ProviderSubcommand<'a>,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
#[serde(tag = "action")]
pub enum ProviderSubcommand<'a> {
    #[serde(rename = "delete")]
    Delete {
        #[serde(rename = "where")]
        #[serde(skip_serializing_if = "Option::is_none")]
        where_clause: Option<&'a str>,
        #[serde(rename = "selectionArgs")]
        #[serde(skip_serializing_if = "Option::is_none")]
        selection_args: Option<&'a [String]>,
    },
    #[serde(rename = "insert")]
    Insert { data: &'a str },
    #[serde(rename = "query")]
    Query {
        #[serde(skip_serializing_if = "Option::is_none")]
        projection: Option<&'a [String]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        selection: Option<&'a str>,
        #[serde(rename = "selectionArgs")]
        #[serde(skip_serializing_if = "Option::is_none")]
        selection_args: Option<&'a [String]>,
        #[serde(rename = "queryArgs")]
        #[serde(skip_serializing_if = "Option::is_none")]
        query_args: Option<&'a str>,
        #[serde(rename = "sortOrder")]
        #[serde(skip_serializing_if = "Option::is_none")]
        sort_order: Option<&'a str>,
    },

    #[serde(rename = "call")]
    Call {
        method: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        arg: Option<&'a str>,
    },

    #[serde(rename = "read")]
    Read,

    #[serde(rename = "write")]
    Write { data: &'a [u8] },
}

#[cfg_attr(test, derive(Debug))]
#[derive(Deserialize)]
struct ServerError {
    err: String,
}

pub const fn pack(a: char, b: char, c: char, d: char) -> u32 {
    (((a as u32) & 0xFF) << 24)
        | (((b as u32) & 0xFF) << 16)
        | (((c as u32) & 0xFF) << 8)
        | ((d as u32) & 0xFF)
}

#[repr(u32)]
pub enum Command {
    Sh = pack('_', '_', 's', 'h'),
    Provider = pack('p', 'r', 'o', 'v'),
    AppService = pack('a', 's', 'v', 'c'),
    SystemService = pack('s', 's', 'v', 'c'),
    Broadcast = pack('b', 'c', 's', 't'),
    StartActivity = pack('_', 'a', 'c', 't'),
    StartService = pack('_', 's', 'v', 'c'),
    RunTest = pack('t', 'e', 's', 't'),
    SystemServiceShellCommand = pack('s', 's', 'h', 'l'),
}

impl AppServer for TcpAppServer {
    fn system_service_shell_cmd(
        &mut self,
        service: &str,
        cmd: Option<&str>,
        timeout: u32,
    ) -> Result<CommandResult> {
        let command = if let Some(cmd) = cmd {
            match split(cmd) {
                None => return Err(Error::InvalidInput(cmd.into())),
                Some(v) => v,
            }
        } else {
            Vec::new()
        };

        let payload = SystemServiceShellCommand {
            service,
            command,
            timeout,
        };
        let res = self.send_command(Command::SystemServiceShellCommand, &payload)?;

        serde_json::from_str(&res).map_err(|e| {
            log::error!("error decoding response {}: {:?}", res, e);
            Error::InvalidResponse
        })
    }

    fn sh_with_shell(&mut self, cmd: &str, shell: Option<&str>) -> Result<CommandResult> {
        let cmd = ShellCommand { cmd, shell };
        let res = self.send_command(Command::Sh, &cmd)?;
        serde_json::from_str(&res).map_err(|e| {
            log::error!("error decoding response {}: {:?}", res, e);
            Error::InvalidResponse
        })
    }

    fn provider_call(&mut self, uri: &str, method: &str, arg: Option<&str>) -> Result<String> {
        let cmd = ProviderSubcommand::Call { method, arg };
        let cmd = ProviderCommand { uri, cmd };
        self.send_command(Command::Provider, &cmd)
    }

    fn provider_delete(
        &mut self,
        uri: &str,
        where_clause: Option<&str>,
        selection_args: Option<&[String]>,
    ) -> Result<i64> {
        let cmd = ProviderSubcommand::Delete {
            where_clause,
            selection_args,
        };
        let cmd = ProviderCommand { uri, cmd };
        let res = self.send_command(Command::Provider, &cmd)?;
        extract_from_json("count", &res, value_to_i64)
    }

    fn provider_insert(&mut self, uri: &str, data: &IntentString) -> Result<Option<String>> {
        let raw_data = data.build();
        let cmd = ProviderSubcommand::Insert { data: &raw_data };
        let cmd = ProviderCommand { uri, cmd };
        let res = self.send_command(Command::Provider, &cmd)?;
        maybe_extract_from_json("uri", &res, value_to_string)
    }

    fn provider_query(
        &mut self,
        uri: &str,
        projection: Option<&[String]>,
        selection: Option<&str>,
        selection_args: Option<&[String]>,
        query_args: Option<&ParcelString>,
        sort_order: Option<&str>,
    ) -> Result<String> {
        let qa = query_args.map(|it| it.build());
        let cmd = ProviderSubcommand::Query {
            projection,
            selection,
            selection_args,
            query_args: qa.as_ref().map(|it| it.as_str()),
            sort_order,
        };
        let cmd = ProviderCommand { uri, cmd };
        self.send_command(Command::Provider, &cmd)
    }

    fn provider_read(&mut self, uri: &str) -> Result<ProviderReadContent> {
        let cmd = ProviderSubcommand::Read;
        let cmd = ProviderCommand { uri, cmd };
        let res = self.send_command(Command::Provider, &cmd)?;
        serde_json::from_str(&res).map_err(|e| {
            log::error!("error decoding response {}: {:?}", res, e);
            Error::InvalidResponse
        })
    }

    fn provider_write(&mut self, uri: &str, data: &[u8]) -> Result<String> {
        let cmd = ProviderSubcommand::Write { data };
        let cmd = ProviderCommand { uri, cmd };
        self.send_command(Command::Provider, &cmd)
    }

    fn call_system_service(
        &mut self,
        name: &str,
        txn: u32,
        iface: Option<&ClassName>,
        parcel_data: Option<&ParcelString>,
    ) -> Result<String> {
        let parcel_data = parcel_data.map(|it| it.build());
        let parcel_data = parcel_data.as_ref().map(|it| it.as_str());
        let payload = CallSystemService {
            name,
            txn,
            iface,
            parcel_data,
        };
        let res = self.send_command(Command::SystemService, &payload)?;
        extract_from_json("response", &res, value_to_string)
    }

    fn call_app_service(
        &mut self,
        txn: u32,
        package: &str,
        class: &ClassName,
        iface: Option<&ClassName>,
        action: Option<&str>,
        parcel_data: Option<&ParcelString>,
    ) -> Result<String> {
        let parcel_data = parcel_data.map(|it| it.build());
        let parcel_data = parcel_data.as_ref().map(|it| it.as_str());
        let payload = CallAppService {
            txn,
            package,
            class,
            iface,
            action,
            parcel_data,
        };
        let res = self.send_command(Command::AppService, &payload)?;
        extract_from_json("response", &res, value_to_string)
    }

    fn broadcast(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<&Vec<String>>,
        intent_data: Option<&IntentString>,
    ) -> Result<String> {
        let intent_data = intent_data.map(IntentString::build);
        let payload = IntentData {
            action,
            data,
            package,
            class,
            flags,
            intent_data: intent_data.as_ref().map(String::as_str),
        };

        self.send_command(Command::Broadcast, &payload)
    }

    fn start_service(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<&Vec<String>>,
        intent_data: Option<&IntentString>,
    ) -> Result<String> {
        let intent_data = intent_data.map(IntentString::build);
        let payload = IntentData {
            action,
            data,
            package,
            class,
            flags,
            intent_data: intent_data.as_ref().map(String::as_str),
        };

        self.send_command(Command::StartService, &payload)
    }

    fn start_activity(
        &mut self,
        action: Option<&str>,
        data: Option<&str>,
        package: Option<&str>,
        class: Option<&str>,
        flags: Option<&Vec<String>>,
        intent_data: Option<&IntentString>,
    ) -> Result<String> {
        let intent_data = intent_data.map(IntentString::build);
        let payload = IntentData {
            action,
            data,
            package,
            class,
            flags,
            intent_data: intent_data.as_ref().map(String::as_str),
        };

        self.send_command(Command::StartActivity, &payload)
    }

    fn run_test(
        &mut self,
        test_name: &str,
        intent_data: Option<&IntentString>,
    ) -> Result<TestOutput> {
        let payload = RunTestPayload {
            test_name,
            intent_data,
        };
        let res = self.send_command(Command::RunTest, &payload)?;
        serde_json::from_str(&res).map_err(|e| {
            log::error!("error decoding response {}: {:?}", res, e);
            Error::InvalidResponse
        })
    }
}

#[derive(Deserialize)]
pub struct ProviderReadContent {
    content: String,
}

impl ProviderReadContent {
    /// Retrieve the data as a base64 encoded string
    pub fn data_b64(&self) -> &str {
        &self.content
    }

    /// Retrieve the data as a lossy UTF8 string
    ///
    /// Returns None of there was no data or if it was invalid base64
    pub fn data_utf8(&self) -> Option<String> {
        if self.content.is_empty() {
            return None;
        }
        let as_bytes = unbase64(&self.content)?;
        Some(String::from_utf8_lossy(&as_bytes).into_owned())
    }

    pub fn data_raw(&self) -> Option<Vec<u8>> {
        unbase64(&self.content)
    }
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
struct RunTestPayload<'a> {
    #[serde(rename = "name")]
    test_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "intentData")]
    intent_data: Option<&'a IntentString<'a>>,
}

impl<'a> Serialize for IntentString<'a> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let as_string = self.build();
        serializer.serialize_str(&as_string)
    }
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
pub struct CallSystemService<'a> {
    pub name: &'a str,
    pub txn: u32,
    #[serde(rename = "interface")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iface: Option<&'a ClassName>,
    #[serde(rename = "parcelData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parcel_data: Option<&'a str>,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
pub struct IntentData<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<&'a str>,
    #[serde(rename = "pkg")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<&'a Vec<String>>,
    #[serde(rename = "intentData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_data: Option<&'a str>,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Serialize)]
pub struct CallAppService<'a> {
    pub txn: u32,
    #[serde(rename = "appId")]
    pub package: &'a str,
    pub class: &'a ClassName,
    #[serde(rename = "interface")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iface: Option<&'a ClassName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'a str>,
    #[serde(rename = "parcelData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parcel_data: Option<&'a str>,
}

pub fn get_server_port(ctx: &dyn Context) -> crate::Result<u16> {
    let port = if ctx.has_env("DTU_SERVER_PORT") {
        let port_string = ctx.unchecked_get_env("DTU_SERVER_PORT");
        match port_string.parse() {
            Ok(v) => v,
            Err(_) => {
                return Err(crate::Error::InvalidEnv(
                    String::from("DTU_SERVER_PORT"),
                    port_string,
                ))
            }
        }
    } else {
        APP_SERVER_PORT
    };

    Ok(port)
}

pub const APP_SERVER_PORT: u16 = 52098;

impl TcpAppServer {
    pub fn default() -> std::result::Result<Self, ConnectError> {
        Self::connect("127.0.0.1", APP_SERVER_PORT)
    }

    pub fn from_ctx(ctx: &dyn Context) -> std::result::Result<Self, ConnectError> {
        let port = if let Ok(v) = get_server_port(ctx) {
            v
        } else {
            return Err(ConnectError::InvalidEnvPort);
        };

        Self::connect("127.0.0.1", port)
    }

    pub fn connect(addr: &str, port: u16) -> std::result::Result<Self, ConnectError> {
        let ip = addr
            .parse::<Ipv4Addr>()
            .map_err(|_| ConnectError::InvalidAddress(addr.to_string()))?;
        let addr = SocketAddr::new(ip.into(), port);
        let stream = TcpStream::connect(addr).map_err(|err| ConnectError::ConnectFailed {
            addr: addr.to_string(),
            port,
            err,
        })?;

        Ok(Self { stream })
    }

    #[inline]
    pub fn send_command<T: Serialize + ?Sized>(
        &mut self,
        cmd: Command,
        payload: &T,
    ) -> Result<String> {
        self.send_raw_command(cmd as u32, payload)
    }

    pub fn send_raw_command<T: Serialize + ?Sized>(
        &mut self,
        cmd: u32,
        payload: &T,
    ) -> Result<String> {
        let serialized = serde_json::to_string(payload)?;
        self.send_raw_command_serialized(cmd, &serialized)
    }

    pub fn send_raw_command_serialized(&mut self, cmd: u32, serialized: &str) -> Result<String> {
        let as_bytes = serialized.as_bytes();
        let len = as_bytes.len();
        let mut header = [0u8; 12];
        encode_header(&mut header, cmd, len as u32);
        log::debug!("sending header {:?}", header);
        self.write_bytes(header.as_slice())?;
        log::debug!("sending payload: {}", serialized);
        self.write_bytes(as_bytes)?;
        self.read_response()
    }

    #[inline]
    pub fn send_command_serialized(&mut self, cmd: Command, serialized: &str) -> Result<String> {
        self.send_raw_command_serialized(cmd as u32, serialized)
    }

    #[inline]
    pub fn transact<T: Serialize + ?Sized, R: DeserializeOwned>(
        &mut self,
        cmd: Command,
        payload: &T,
    ) -> Result<R> {
        self.transact_raw_command(cmd as u32, payload)
    }

    pub fn transact_raw_command<T: Serialize + ?Sized, R: DeserializeOwned>(
        &mut self,
        cmd: u32,
        payload: &T,
    ) -> Result<R> {
        let res = self.send_raw_command(cmd, payload)?;
        serde_json::from_str(&res).map_err(|e| {
            log::error!("error decoding response {}: {:?}", res, e);
            Error::InvalidResponse
        })
    }

    fn read_response(&mut self) -> Result<String> {
        let mut header = [0u8; 12];
        self.stream.read_exact(header.as_mut_slice())?;
        log::trace!("header: {:?}", header);
        let stat = Status::from_bytes([header[0], header[1], header[2], header[3]])?;

        let len_str_bytes = &[
            header[4], header[5], header[6], header[7], header[8], header[9], header[10],
            header[11],
        ];
        let len_str = std::str::from_utf8(len_str_bytes).map_err(|e| {
            log::error!("getting len str {:?} {:?}", len_str_bytes, e);
            Error::InvalidResponse
        })?;
        log::trace!("raw len str: {}", len_str);

        let len = u32::from_str_radix(len_str, 16).map_err(|e| {
            log::error!("parsing len str {} {:?}", len_str, e);
            Error::InvalidResponse
        })?;

        log::trace!("reading {} bytes from server", len);

        let mut data = String::with_capacity(len as usize);
        self.stream.read_to_string(&mut data)?;
        if let Status::Fail = stat {
            let err: ServerError = serde_json::from_str(&data).map_err(|e| {
                log::error!("error response {} wasn't valid {:?}", data, e);
                Error::InvalidResponse
            })?;
            return Err(Error::ServerError(err.err));
        }
        log::debug!("json response: {}", data);
        Ok(data)
    }

    fn write_bytes(&mut self, raw: &[u8]) -> Result<()> {
        self.stream.write_all(raw)?;
        Ok(())
    }
}

#[repr(u32)]
#[cfg_attr(test, derive(PartialEq, Debug))]
enum Status {
    Ok = pack('G', 'O', 'O', 'D'),
    Fail = pack('F', 'A', 'I', 'L'),
}

impl Status {
    fn from_bytes(bytes: [u8; 4]) -> Result<Self> {
        let as_u32 = u32::from_be_bytes(bytes);
        if as_u32 == (Status::Ok as u32) {
            return Ok(Status::Ok);
        } else if as_u32 == (Status::Fail as u32) {
            return Ok(Status::Fail);
        }
        Err(Error::InvalidResponse)
    }
}

fn encode_header(into: &mut [u8; 12], cmd: u32, payload_len: u32) {
    let mut idx = 0;
    let mut shift = 28;
    // The command is always just ascii
    for b in (cmd as u32).to_be_bytes() {
        into[idx] = b;
        idx += 1;
    }
    // Encode the len as hex:
    // 00 00 00 00
    for _ in 0..8 {
        let sel = (payload_len >> shift) & 0xF;
        into[idx] = HEX_BYTES[sel as usize];
        shift -= 4;
        idx += 1;
    }
}

fn value_to_string(v: Value) -> Option<String> {
    v.as_str().map(String::from)
}

fn value_to_i64(v: Value) -> Option<i64> {
    v.as_i64()
}

pub fn maybe_extract_int_from_json(key: &str, json: &str) -> Result<Option<i64>> {
    maybe_extract_from_json(key, json, value_to_i64)
}

pub fn maybe_extract_string_from_json(key: &str, json: &str) -> Result<Option<String>> {
    maybe_extract_from_json(key, json, value_to_string)
}

pub fn maybe_extract_from_json<F, T>(key: &str, json: &str, transform: F) -> Result<Option<T>>
where
    F: FnOnce(Value) -> Option<T>,
{
    let reader = StrRead::new(json);
    let mut des = serde_json::Deserializer::new(reader);
    let mut value = Value::deserialize(&mut des)?;

    let val = value.get_mut(key).map(|it| it.take());

    match val {
        None => Ok(None),
        Some(v) => Ok(transform(v)),
    }
}

pub fn extract_int_from_json(key: &str, json: &str) -> Result<i64> {
    extract_from_json(key, json, value_to_i64)
}

pub fn extract_string_from_json(key: &str, json: &str) -> Result<String> {
    extract_from_json(key, json, value_to_string)
}

pub fn extract_from_json<F, T>(key: &str, json: &str, transform: F) -> Result<T>
where
    F: FnOnce(Value) -> Option<T>,
{
    maybe_extract_from_json(key, json, transform)?.ok_or_else(|| {
        log::error!(
            "error decoding response {}: missing {} or wrong type",
            json,
            key
        );
        Error::InvalidResponse
    })
}

struct Base64Visitor;

impl<'de> Visitor<'de> for Base64Visitor {
    type Value = Vec<u8>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("string containing base64 encoded bytes")
    }

    fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match unbase64(v) {
            Some(vec) => Ok(vec),
            None => Err(E::custom(format!("{} is not valid base64", v))),
        }
    }
}

fn deserialize_b64<'de, D>(deser: D) -> std::result::Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let visitor = Base64Visitor;
    deser.deserialize_str(visitor)
}

#[cfg(test)]
mod test {
    use super::*;

    macro_rules! serialize_test {
        (
            $value:expr,
            $expected:literal
        ) => {
            let val = $value;
            let js = serde_json::to_string(&val)
                .unwrap_or_else(|e| format!("failed to serialize {:?}: {}", val, e));
            assert_eq!(
                js.as_str(),
                $expected,
                "{:?} JSON serialization was incorrect",
                val
            );
        };
    }

    #[test]
    fn test_status() {
        let stat = Status::from_bytes([0x47, 0x4f, 0x4f, 0x44]);
        assert_eq!(stat.unwrap(), Status::Ok);

        let stat = Status::from_bytes([0x46, 0x41, 0x49, 0x4c]);
        assert_eq!(stat.unwrap(), Status::Fail);

        let stat = Status::from_bytes([0x00, 0x00, 0x00, 0x00]);
        assert!(stat.is_err())
    }

    #[test]
    fn test_encode_header() {
        let cmd = Command::Sh;
        let payload_len: u32 = 0xCAFEC0DE;
        let mut into = [0u8; 12];
        encode_header(&mut into, cmd as u32, payload_len);
        assert_eq!(
            into.as_slice(),
            &[b'_', b'_', b's', b'h', b'c', b'a', b'f', b'e', b'c', b'0', b'd', b'e',],
            "encode failed"
        );
    }

    #[test]
    fn test_provider_command() {
        serialize_test!(
            ProviderCommand {
                uri: "content://neato",
                cmd: ProviderSubcommand::Call {
                    method: "target",
                    arg: Some("such arg string")
                }
            },
            r#"{"uri":"content://neato","action":"call","method":"target","arg":"such arg string"}"#
        );
    }

    #[test]
    fn test_json_shell_cmd() {
        serialize_test!(
            ShellCommand {
                cmd: "id",
                shell: None,
            },
            r#"{"cmd":"id"}"#
        );
        serialize_test!(
            ShellCommand {
                cmd: "id",
                shell: Some("/system/bin/sh"),
            },
            r#"{"cmd":"id","shell":"/system/bin/sh"}"#
        );
    }
}
