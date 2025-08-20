use clap::builder::{NonEmptyStringValueParser, TypedValueParser};

use crate::parsers::simple_error;
use regex::Regex;

#[derive(Clone)]
pub struct HeapSizeValueParser;

impl TypedValueParser for HeapSizeValueParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let parser = NonEmptyStringValueParser::new();
        let val = parser.parse_ref(cmd, arg, value)?;
        let re = Regex::new(r"^\d+[m|g|mb|gb|M|G|GB|MB]$");
    
        if re.map_or(false, |re| re.is_match(val.as_str())) {
            Ok(val)
        } else {
            Err(simple_error("provided `heap_size` was not formatted correctly! (format must match regex `^\\d+[m|g|mb|gb|M|G|GB|MB]$`)"))
        }
    }
}