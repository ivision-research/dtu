use crate::parsers::simple_error;
use crate::utils::prompt_choice;
use clap::builder::{NonEmptyStringValueParser, TypedValueParser};
use dtu::db::device::models::Apk;
use dtu::db::meta::get_default_metadb;
use dtu::db::{DeviceDatabase, MetaDatabase, MetaSqliteDatabase};
use dtu::prereqs::Prereq;
use dtu::utils::DevicePath;
use dtu::{DefaultContext, REPLACED_DEVICE_PATH_SEP};

#[derive(Clone)]
pub struct DevicePathValueParser;

pub fn device_path_arg_parse(
    db: &DeviceDatabase,
    val: &str,
) -> Result<Option<DevicePath>, clap::Error> {
    if val.starts_with(REPLACED_DEVICE_PATH_SEP) {
        return Ok(Some(DevicePath::from_squashed(val)));
    }

    let is_apk = val.ends_with(".apk");

    // We do it this way in the off chance that the `*.apk` is the same
    let apks = db
        .get_apks()
        .map_err(simple_error)?
        .into_iter()
        .filter_map(|it| {
            if is_apk {
                if it.name == val {
                    Some(it.device_path)
                } else {
                    None
                }
            } else if it.app_name == val {
                Some(it.device_path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if apks.is_empty() {
        return Ok(None);
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

    Ok(Some(apk))
}

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
        let meta = get_default_metadb(&ctx).map_err(simple_error)?;
        meta.ensure_prereq(Prereq::SQLDatabaseSetup)
            .map_err(simple_error)?;
        let db = DeviceDatabase::new(&ctx).map_err(simple_error)?;

        match device_path_arg_parse(&db, &val)? {
            Some(v) => Ok(v),
            None => Err(simple_error(format!("no APK found matching {val}"))),
        }
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
        let db = DeviceDatabase::new(&ctx).map_err(simple_error)?;
        Ok(db.get_apk_by_apk_name(&val).map_err(simple_error)?)
    }
}
