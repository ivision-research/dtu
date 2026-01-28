#[macro_use]
mod common;
pub use common::{
    ApkComponent, ApkIPC, ApkIPCKind, Enablable, Error, Exportable, Idable, PermissionMode,
    PermissionProtected, Result,
};

pub mod device;
pub mod meta;

#[cfg(feature = "graph")]
pub mod graph;

pub use device::db::DeviceDatabase;
pub use meta::db::{Database as MetaDatabase, MetaSqliteDatabase};
pub use meta::DefaultMetaDatabase;
