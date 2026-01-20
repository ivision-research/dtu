use clap::builder::{NonEmptyStringValueParser, TypedValueParser};

use crate::parsers::simple_error;
use dtu::db::device::models::Provider;
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::DefaultContext;

#[allow(dead_code)]
#[derive(Clone)]
pub struct ProviderValueParser;

impl TypedValueParser for ProviderValueParser {
    type Value = Provider;

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
        let prov = db
            .get_provider_containing_authority(&val)
            .map_err(simple_error)?;
        Ok(prov)
    }
}
