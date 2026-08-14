mod common;
pub use common::{DatabaseId, Error, Idable, Result};

pub use device::{
    ApkComponent, ApkIPC, ApkIPCKind, Diffable, Enablable, Exportable, PermissionMode,
    PermissionProtected,
};

pub(crate) use common::query;

pub mod device;
pub mod meta;

#[cfg(feature = "graph")]
pub mod graph;

pub use device::db::DeviceDatabase;
pub use meta::db::{Database as MetaDatabase, MetaSqliteDatabase};
pub use meta::DefaultMetaDatabase;
