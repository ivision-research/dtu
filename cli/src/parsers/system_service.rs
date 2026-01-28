use clap::builder::{NonEmptyStringValueParser, TypedValueParser};

use crate::parsers::simple_error;
use dtu::db::device::models::SystemService;
use dtu::db::{DeviceDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[derive(Clone)]
pub struct SystemServiceValueParser;

impl TypedValueParser for SystemServiceValueParser {
    type Value = SystemService;

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
        let db = DeviceDatabase::new(&ctx).map_err(simple_error)?;
        Ok(db.get_system_service_by_name(&val).map_err(simple_error)?)
    }
}
