use std::io;

use thiserror;
use zip::result::ZipError;

pub type DecompileResult<T> = Result<T, DecompileError>;

#[derive(thiserror::Error, Debug)]
pub enum DecompileError {
    #[error("{0}")]
    IO(io::Error),
    #[error("{0}")]
    PrereqError(crate::Error),
    #[error("decompile source file doesn't exist")]
    SourceFileMissing,
    #[error("invalid file type for decompilation")]
    InvalidFile,
}

impl From<io::Error> for DecompileError {
    fn from(err: io::Error) -> Self {
        Self::IO(err)
    }
}

impl From<crate::Error> for DecompileError {
    fn from(err: crate::Error) -> Self {
        Self::PrereqError(err)
    }
}

impl From<ZipError> for DecompileError {
    fn from(err: ZipError) -> Self {
        match err {
            ZipError::Io(io) => Self::from(io),
            ZipError::FileNotFound => Self::SourceFileMissing,
            _ => Self::InvalidFile,
        }
    }
}
