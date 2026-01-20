use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[cfg(any(feature = "sql", feature = "graph"))]
use crate::db;
#[cfg(feature = "sql")]
use crate::prereqs::Prereq;
use crate::utils::path_must_str;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("required binary `{0}` not available to context")]
    MissingBin(String),
    #[error("missing required env var: {0}")]
    MissingEnv(String),

    #[cfg(feature = "sql")]
    #[error("unsatisfied prerequisite: {0:?}")]
    UnsatisfiedPrereq(Prereq),

    #[error("{0}")]
    IO(io::Error),

    #[error("invalid env var {0} ({1})")]
    InvalidEnv(String, String),

    #[error("command failed with status {0}: {1}")]
    CommandError(i32, String),

    #[error("task was cancelled by user")]
    Cancelled,

    #[error("failed to get basedirs")]
    NoBaseDirs,

    #[error("no adb device connected")]
    NoAdbDevice,
    #[error("adb device {0} not found")]
    AdbDeviceNotFound(String),
    #[error("adb disabled by configuration file")]
    AdbDisabled,

    #[error("bad path {0:?}")]
    BadPath(PathBuf),

    #[error("generic error: {0}")]
    Generic(String),

    #[error("task already done")]
    TaskAlreadyDone,

    #[error("invalid config {0}: {1}")]
    InvalidConfig(String, String),

    #[error("file {0} doesn't exist")]
    MissingFile(String),
}

impl Error {
    pub fn new_generic<S: ToString + ?Sized>(s: &S) -> Self {
        Self::Generic(s.to_string())
    }

    pub fn new_cfg<S: ToString + ?Sized>(path: &Path, s: &S) -> Self {
        let as_str = path_must_str(path.as_ref());
        Self::InvalidConfig(as_str.into(), s.to_string())
    }
}

#[cfg(feature = "sql")]
impl From<db::Error> for Error {
    fn from(value: db::Error) -> Self {
        Self::Generic(value.to_string())
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for Error {
    fn from(value: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self::Generic(value.to_string())
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::IO(err)
    }
}
