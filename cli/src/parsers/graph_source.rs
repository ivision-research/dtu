use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use dtu::{
    db::{graph::FRAMEWORK_SOURCE, meta::get_default_metadb, DeviceDatabase, MetaDatabase},
    prereqs::Prereq,
    DefaultContext,
};

use crate::parsers::{apk::device_path_arg_parse, simple_error};

#[allow(dead_code)]
#[derive(Clone)]
pub struct GraphSourceValueParser;

impl TypedValueParser for GraphSourceValueParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let parser = NonEmptyStringValueParser::new();
        let val = parser.parse_ref(cmd, arg, value)?;

        // Framework match directly just short circuit
        if val == FRAMEWORK_SOURCE {
            return Ok(val);
        }

        let ctx = DefaultContext::new();
        let meta = get_default_metadb(&ctx).map_err(simple_error)?;

        if matches!(meta.prereq_done(Prereq::SQLDatabaseSetup), Ok(true)) {
            let db = DeviceDatabase::new(&ctx).map_err(simple_error)?;
            // Check to see if it is an APK
            let path = device_path_arg_parse(&db, &val)?;

            if let Some(p) = path {
                return Ok(p.into_squashed());
            }
        }

        // Otherwise just return the value, no need to enforce anything here I guess
        Ok(val)
    }
}
