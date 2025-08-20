use super::models::{ClassCallPath, ClassMeta, ClassSourceCallPath, MethodCallSearch, MethodMeta};
use crate::utils::ClassName;
use crate::Context;
use dtu_proc_macro::wraps_base_error;
use std::collections::BTreeSet;
use std::io::{self, Write};

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
    fn initialize(&self) -> Result<()>;

    /// Optimize the database, defaults to a NOOP but may be supported by
    /// some backends
    fn optimize(&self) -> Result<()> {
        log::warn!("Database implementation doesn't support optimization");
        Ok(())
    }

    fn get_all_sources(&self) -> Result<BTreeSet<String>>;

    /// Find all child classes of the given parent class
    fn find_child_classes_of(
        &self,
        parent: &ClassName,
        source: Option<&str>,
    ) -> Result<Vec<ClassMeta>>;

    /// Find all classes that implement the given interface
    fn find_classes_implementing(
        &self,
        iface: &ClassName,
        source: Option<&str>,
    ) -> Result<Vec<ClassMeta>>;

    /// Find all callers of the given method
    ///
    /// Depth specifies the call depth, for example:
    ///
    /// - depth = 1 will only find immediate calls
    /// - depth = 2 will find calls that call something that calls the method
    ///
    /// and so on. A high depth value will make this call take a long time and
    /// generally a lot of indirection will cause noise in the output, as each
    /// method call further away you are the more the input can be transformed
    /// before the call you're interested in.
    ///
    /// Generally, I wouldn't go above depth = 3 for good results.
    fn find_callers(
        &self,
        method: &MethodCallSearch,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<ClassSourceCallPath>>;

    /// Find all calls leaving the given method up to a given depth.
    fn find_outgoing_calls(
        &self,
        from: &MethodMeta,
        source: &str,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<Vec<ClassCallPath>>;

    /// Get all classes defined by the given source
    fn get_classes_for(&self, source: &str) -> Result<Vec<ClassName>>;

    /// Get all methods defined by the given soruce
    fn get_methods_for(&self, source: &str) -> Result<Vec<MethodMeta>>;

    /// Wipe the database
    fn wipe(&self, ctx: &dyn Context) -> Result<()>;

    /// Remove all references to the given source from the database
    fn remove_source(&self, source: &str) -> Result<()>;

    /// Adds the contents of a given directory to the graph.
    #[cfg(feature = "setup")]
    fn add_directory(
        &self,
        ctx: &dyn Context,
        opts: super::setup::AddDirectoryOptions,
        monitor: &dyn crate::tasks::EventMonitor<super::setup::SetupEvent>,
        cancel: &crate::tasks::TaskCancelCheck,
    ) -> Result<()>;

    /// Used for REPL implementation
    fn eval(&self, script: &str, writer: &mut dyn Write) -> Result<()>;
}
