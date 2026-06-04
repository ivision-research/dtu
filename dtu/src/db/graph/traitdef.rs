use std::collections::HashSet;

use crate::utils::ClassName;
use crate::Context;

use super::common::Result;
use super::models::*;

pub const FRAMEWORK_SOURCE: &'static str = "framework";

pub enum StringSearch<'a> {
    Exact(&'a str),
    Like(&'a str),
}

impl<'a> From<&'a String> for StringSearch<'a> {
    fn from(value: &'a String) -> Self {
        Self::from(value.as_str())
    }
}

impl<'a> From<&'a str> for StringSearch<'a> {
    fn from(value: &'a str) -> Self {
        if value.contains("%") {
            Self::Like(value)
        } else {
            Self::Exact(value)
        }
    }
}

/// Trait for interfacing with the graph database. The graph database is used
/// for finding relationships in the analyzed smali files.
///
/// The Framework and each APK have their own database
pub trait GraphDatabase: Sync + Send {
    /// Get all source names in the database
    fn get_all_sources(&self) -> Result<HashSet<String>>;

    /// Find all methods matching the given search critera
    fn get_methods(&self, search: &MethodSearch) -> Result<Vec<MethodSpec>>;

    /// Find all methods matching the given search criteria returning only the database IDs
    fn get_method_ids(&self, search: &MethodSearch) -> Result<Vec<i32>> {
        Ok(self
            .get_methods(search)?
            .into_iter()
            .map(|it| it.id)
            .collect::<Vec<_>>())
    }

    /// Get all fields matching the given search criteria
    fn get_fields(&self, search: &FieldSearch) -> Result<Vec<FieldSpec>>;

    /// Get all fields matching the given search criteria returning only the database IDs
    fn get_field_ids(&self, search: &FieldSearch) -> Result<Vec<i32>> {
        Ok(self
            .get_fields(search)?
            .into_iter()
            .map(|it| it.id)
            .collect::<Vec<_>>())
    }

    /// Find all parent classes of the given child class
    ///
    /// The source is not optional here, as only a single class should be
    /// queried for this to make sense
    fn find_parent_classes_of(&self, child: &ClassName, source: &str) -> Result<Vec<ClassSpec>>;

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
    fn find_callers(
        &self,
        method: &MethodSearch,
        call_source: Option<&str>,
        depth: usize,
    ) -> Result<Vec<MethodCallPath>>;

    /// Find all calls leaving the given method up to a given depth.
    fn find_outgoing_calls(&self, from: &MethodSearch, depth: usize)
        -> Result<Vec<MethodCallPath>>;

    /// Find all classes with the given method
    fn find_classes_with_method(
        &self,
        name: &str,
        args: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<ClassSpec>>;

    /// Get all methods referencing the given field
    fn get_methods_referencing_field(
        &self,
        field: i32,
        action: Option<FieldAccessOp>,
    ) -> Result<Vec<MethodSpec>>;

    /// Get all fields referenced by the given method
    fn get_method_field_refs(&self, method: i32) -> Result<Vec<FieldRef>>;

    /// Get all constant strings discovered in the method
    fn get_strings_for_method(&self, method: i32) -> Result<Vec<String>>;

    /// Get all constant strings discovered in the given source
    fn get_strings_for_source(&self, source: &str) -> Result<Vec<String>>;

    /// Find all strings that match the given search parameters
    fn find_strings(
        &self,
        string: StringSearch,
        source: Option<&str>,
    ) -> Result<Vec<SourcedString>>;

    /// Get all methods that contain a string matching the provided search params
    fn get_methods_for_string(&self, string: StringSearch) -> Result<Vec<MethodSpec>>;

    /// Get all classes defined by the given source
    fn get_classes_for(&self, source: &str) -> Result<Vec<ClassName>>;

    /// Get all methods defined by the given source
    fn get_methods_for(&self, source: &str) -> Result<Vec<MethodSpec>>;

    /// Wipe the database
    fn wipe(&self, ctx: &dyn Context) -> Result<()>;

    /// Remove all references to the given source from the database
    fn remove_source(&self, source: &str) -> Result<()>;
}
