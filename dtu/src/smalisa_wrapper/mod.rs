use std::{fs::File, io};

mod gen_csvs;
pub use gen_csvs::{write_analysis_files, Event, CSV};

use smalisa::{LexError, ParseError};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    IO(io::Error),

    #[error("{0}")]
    Base(crate::Error),

    /// Since there is no way to actually handle these errors, just wrap
    /// them up as a String.
    #[error("{0}")]
    Smalisa(String),

    #[error("{0}")]
    Generic(String),

    #[error("task cancelled by user")]
    Cancelled,
}

impl From<crate::Error> for Error {
    fn from(value: crate::Error) -> Self {
        Self::Base(value)
    }
}

impl<'a> From<ParseError<'a>> for Error {
    fn from(value: ParseError) -> Self {
        Self::Smalisa(value.to_string())
    }
}

impl<'a> From<LexError<'a>> for Error {
    fn from(value: LexError) -> Self {
        Self::Smalisa(value.to_string())
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::IO(value)
    }
}

pub(crate) fn arena_for_file(file: &File, def: Option<usize>) -> smalisa::Arena {
    let size = file
        .metadata()
        .ok()
        .and_then(|it| usize::try_from(it.len()).ok())
        .unwrap_or(def.unwrap_or(1 * 1024));
    smalisa::Arena::with_capacity(size)
}
