use clap::builder::{NonEmptyStringValueParser, TypedValueParser};

use crate::parsers::simple_error;
use dtu::db;
use dtu::db::meta::models::AppActivity;
use dtu::db::{MetaDatabase, MetaSqliteDatabase};
use dtu::DefaultContext;

#[derive(Clone)]
pub struct AppActivityValueParser;

impl TypedValueParser for AppActivityValueParser {
    type Value = AppActivity;

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
        let act = match meta.get_app_activity_by_name(val.as_str()) {
            Err(db::Error::NotFound) => {
                let activities = meta.get_app_activities().map_err(simple_error)?;
                let mut opts = String::new();
                for a in activities {
                    opts.push_str("- ");
                    opts.push_str(a.name.as_str());
                    opts.push('\n');
                }
                let err = format!("no activity named {} options are\n{}", val, opts);
                return Err(simple_error(err));
            }
            Err(e) => return Err(simple_error(e.to_string())),
            Ok(it) => it,
        };
        Ok(act)
    }
}
