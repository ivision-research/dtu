#[macro_use]
mod common;
pub use common::{
    ApkComponent, ApkIPC, ApkIPCKind, Enablable, Error, Exportable, Idable, PermissionMode,
    PermissionProtected, Result,
};

pub mod device;
pub mod meta;

pub use device::db::{Database as DeviceDatabase, DeviceSqliteDatabase};
pub use meta::db::{Database as MetaDatabase, MetaSqliteDatabase};
