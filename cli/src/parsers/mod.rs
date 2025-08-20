use clap::error::ErrorKind;
use std::fmt::Display;

mod apk;
pub use apk::{ApkValueParser, DevicePathValueParser};

mod system_service;
pub use system_service::SystemServiceValueParser;

mod app_activity;
pub use app_activity::AppActivityValueParser;

mod provider;
//pub use provider::ProviderValueParser;

#[cfg(feature = "neo4j")]
mod heap_size;
#[cfg(feature = "neo4j")]
pub use heap_size::HeapSizeValueParser;

mod diff_source;
pub use diff_source::DiffSourceValueParser;

mod parcel_string_parser;
pub use parcel_string_parser::{parse_intent_string, parse_parcel_string};

pub fn simple_error(err: impl Display) -> clap::Error {
    clap::Error::raw(ErrorKind::InvalidValue, format!("{}\n", err.to_string()))
}
