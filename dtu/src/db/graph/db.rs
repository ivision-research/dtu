use super::models::{ClassSpec, MethodCallPath, MethodSpec};
use crate::db::graph::models::{ClassSearch, MethodSearch};
use crate::utils::ClassName;
use crate::Context;
use dtu_proc_macro::wraps_base_error;
use std::collections::HashSet;
use std::io;

pub const FRAMEWORK_SOURCE: &'static str = "framework";

#[wraps_base_error]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    IO(io::Error),

    #[error("error communicating with database")]
    ConnectionError,

    #[error("row is missing field {0}")]
    MissingField(String),

    #[error("failed to update meta database")]
    MetaDBUpdateFailed,

    #[error("generic error {0}")]
    Generic(String),

    #[error("user cancelled task")]
    Cancelled,

    #[error("method {0} not supported by graph implementation")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Trait for interfacing with the graph database. The graph database is used
/// for finding relationships in the analyzed smali files.
///
/// The Framework and each APK have their own database
pub trait GraphDatabase: Sync + Send {
    /// Get all source names in the database
    fn get_all_sources(&self) -> Result<HashSet<String>>;

    /// Find all child classes of the given parent class
    ///
    /// The source is for the source in which the relationship was discovered,
    /// which will be the child class's source: this may differ from the parent's
    /// source.
    fn find_child_classes_of(
        &self,
        parent: &ClassSearch,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>>;

    /// Find all classes that implement the given interface
    ///
    /// The source is for the source in which the relationship was discovered,
    /// which will be the implementing class's source: this may differ from the
    /// interface definition's source.
    fn find_classes_implementing(
        &self,
        iface: &ClassSearch,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>>;

    /// Find all callers of the given method
    ///
    /// Depth specifies the call depth, for example:
    ///
    /// - depth = 1 will only find immediate calls
    /// - depth = 2 will find calls that call something that calls the method
    ///
    /// and so on.
    fn find_callers(&self, method: &MethodSearch, call_source: Option<&str>, depth: usize) -> Result<Vec<MethodCallPath>>;

    /// Find all calls leaving the given method up to a given depth.
    fn find_outgoing_calls(&self, from: &MethodSearch, depth: usize)
        -> Result<Vec<MethodCallPath>>;

    /// Get all classes defined by the given source
    fn get_classes_for(&self, source: &str) -> Result<Vec<ClassName>>;

    /// Get all methods defined by the given soruce
    fn get_methods_for(&self, source: &str) -> Result<Vec<MethodSpec>>;

    /// Wipe the database
    fn wipe(&self, ctx: &dyn Context) -> Result<()>;

    /// Remove all references to the given source from the database
    fn remove_source(&self, source: &str) -> Result<()>;
}
