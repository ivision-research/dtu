use crate::parsers::simple_error;
use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use dtu::db::sql::device::models::DiffSource;
use dtu::db::sql::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[derive(Clone)]
pub struct DiffSourceValueParser;

impl TypedValueParser for DiffSourceValueParser {
    type Value = DiffSource;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let parser = NonEmptyStringValueParser::new();
        let val = parser.parse_ref(cmd, arg, value)?;

        let ctx = DefaultContext::new();
        let meta = MetaSqliteDatabase::new(&ctx).map_err(simple_error)?;
        meta.ensure_prereq(Prereq::SQLDatabaseSetup)
            .map_err(simple_error)?;
        let db = DeviceSqliteDatabase::new(&ctx).map_err(simple_error)?;
        Ok(db.get_diff_source_by_name(&val).map_err(simple_error)?)
    }
}
