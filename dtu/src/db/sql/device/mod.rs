pub mod db;
pub mod models;
pub mod schema;

pub use db::EMULATOR_DIFF_SOURCE;

#[cfg(feature = "setup")]
pub mod diff;
#[cfg(feature = "setup")]
pub use diff::*;

#[cfg(feature = "setup")]
pub mod setup;
#[cfg(feature = "setup")]
pub use setup::*;

use super::common;

/// Get the default [DeviceDatabase] implementation
pub fn get_default_devicedb(ctx: &dyn crate::Context) -> common::Result<db::DeviceSqliteDatabase> {
    db::DeviceSqliteDatabase::new(ctx)
}
