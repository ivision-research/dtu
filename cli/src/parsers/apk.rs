use crate::parsers::simple_error;
use crate::utils::{find_fully_qualified_apk, prompt_choice};
use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use dtu::db::device::models::Apk;
use dtu::db::{DeviceDatabase, DeviceSqliteDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::DevicePath;
use dtu::DefaultContext;

#[derive(Clone)]
pub struct DevicePathValueParser;

impl TypedValueParser for DevicePathValueParser {
    type Value = DevicePath;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let parser = NonEmptyStringValueParser::new();
        let val = parser.parse_ref(cmd, arg, value)?;

        let ctx = DefaultContext::new();
        let apks = find_fully_qualified_apk(&ctx, &val).map_err(simple_error)?;
        if apks.is_empty() {
            return Err(simple_error(format!("no apks matching {}", val)));
        }
        let apk = if apks.len() > 1 {
            prompt_choice(
                &apks,
                &format!("Multiple APKs found matching {}:", val),
                "APK number: ",
            )
            .map_err(|e| simple_error(e.to_string()))?
        } else {
            apks.get(0).unwrap()
        }
        .clone();

        Ok(apk)
    }
}

#[derive(Clone)]
pub struct ApkValueParser;

impl TypedValueParser for ApkValueParser {
    type Value = Apk;

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
        Ok(db.get_apk_by_apk_name(&val).map_err(simple_error)?)
    }
}
