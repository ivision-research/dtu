use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use crate::utils::bytes_to_hex;

#[cfg_attr(test, derive(Debug))]
#[derive(Clone)]
pub enum ParcelStringElem<'a> {
    Long(i64),
    Int(i32),
    Short(i16),
    Byte(u8),
    Bool(bool),
    Double(f64),
    Float(f32),
    Binder,
    Null,
    WriteFd(Cow<'a, str>),
    ReadFd(Cow<'a, str>),
    String(Cow<'a, str>),
    RawByteArray(Cow<'a, [u8]>),
    HexByteArray(Cow<'a, str>),
    Bundle(HashMap<String, ParcelStringElem<'a>>),
    Map(HashMap<ParcelStringElem<'a>, ParcelStringElem<'a>>),
    List(Vec<ParcelStringElem<'a>>),
    Complex(Vec<ParcelStringElem<'a>>),
    Message(i32, i32, i32, Option<HashMap<String, ParcelStringElem<'a>>>),
}

/// Builder object to create Parcel strings
#[derive(Eq)]
pub struct ParcelString<'a> {
    elems: Vec<ParcelStringElem<'a>>,
}

impl<'a> Clone for ParcelString<'a> {
    fn clone(&self) -> Self {
        let mut elems = Vec::with_capacity(self.elems.len());
        for e in &self.elems {
            elems.push(e.clone())
        }
        Self { elems }
    }
}

impl<'a> Hash for ParcelString<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for e in &self.elems {
            let as_str = e.to_string();
            state.write(as_str.as_bytes())
        }
    }
}

impl<'a> PartialEq for ParcelString<'a> {
    fn eq(&self, other: &Self) -> bool {
        if self.elems.len() != other.elems.len() {
            return false;
        }

        for (i, s) in self.elems.iter().enumerate() {
            if s != &other.elems[i] {
                return false;
            }
        }
        true
    }
}

impl<'a> Default for ParcelString<'a> {
    fn default() -> Self {
        Self { elems: Vec::new() }
    }
}

impl<'a> Display for ParcelString<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.build())
    }
}

macro_rules! def_write_fmt {
    ($name:ident, $ty:ident) => {
        pub fn $name(&mut self) -> &mut Self {
            self.push(ParcelStringElem::$ty)
        }
    };
    ($name:ident, $ty:ident, $inner_ty:ty) => {
        pub fn $name(&mut self, val: $inner_ty) -> &mut Self {
            self.push(ParcelStringElem::$ty(val))
        }
    };
}

impl<'a> ParcelString<'a> {
    pub fn build(&self) -> String {
        let mut into = String::new();
        self.build_to(&mut into);
        into
    }

    fn build_to(&self, into: &mut String) {
        let last = self.elems.len() - 1;
        for (i, e) in self.elems.iter().enumerate() {
            let as_str = e.to_string();
            into.push_str(&as_str);
            if i < last {
                into.push(',');
            }
        }
    }

    pub fn clear(&mut self) {
        self.elems.clear();
    }

    def_write_fmt!(write_long, Long, i64);
    def_write_fmt!(write_int, Int, i32);
    def_write_fmt!(write_short, Short, i16);
    def_write_fmt!(write_byte, Byte, u8);
    def_write_fmt!(write_bool, Bool, bool);
    def_write_fmt!(write_double, Double, f64);
    def_write_fmt!(write_float, Float, f32);

    def_write_fmt!(write_binder, Binder);
    def_write_fmt!(write_null, Null);

    pub fn add_message(
        &mut self,
        what: i32,
        arg1: i32,
        arg2: i32,
        bund: Option<HashMap<String, ParcelStringElem<'a>>>,
    ) -> &mut Self {
        self.push(ParcelStringElem::Message(what, arg1, arg2, bund))
    }

    pub fn add_write_fd(&mut self, val: &'a str) -> &mut Self {
        self.push(ParcelStringElem::WriteFd(val.into()))
    }

    pub fn add_read_fd(&mut self, val: &'a str) -> &mut Self {
        self.push(ParcelStringElem::ReadFd(val.into()))
    }

    pub fn write_string(&mut self, val: &'a str) -> &mut Self {
        self.push(ParcelStringElem::String(val.into()))
    }

    pub fn write_hex_bytes(&mut self, hex: &'a str) -> &mut Self {
        self.push(ParcelStringElem::HexByteArray(hex.into()))
    }

    pub fn write_bytes(&mut self, value: &'a [u8]) -> &mut Self {
        self.push(ParcelStringElem::RawByteArray(value.into()))
    }

    pub fn write_bundle(&mut self, map: HashMap<String, ParcelStringElem<'a>>) -> &mut Self {
        self.push(ParcelStringElem::Bundle(map))
    }

    pub fn write_map(
        &mut self,
        map: HashMap<ParcelStringElem<'a>, ParcelStringElem<'a>>,
    ) -> &mut Self {
        self.push(ParcelStringElem::Map(map))
    }

    pub fn write_list(&mut self, lst: Vec<ParcelStringElem<'a>>) -> &mut Self {
        self.push(ParcelStringElem::List(lst))
    }

    pub fn write_complex_type(&mut self, lst: Vec<ParcelStringElem<'a>>) -> &mut Self {
        self.push(ParcelStringElem::Complex(lst))
    }

    #[inline]
    pub fn push(&mut self, e: ParcelStringElem<'a>) -> &mut Self {
        self.elems.push(e);
        self
    }
}

pub(crate) fn escape_string(s: &str) -> String {
    s.escape_default()
        .to_string()
        .replace("=", "\\=")
        .replace(",", "\\,")
        .replace(":", "\\:")
        .replace("|", "\\|")
        .replace(">", "\\>")
        .replace("%", "\\%")
}

impl<'a> Eq for ParcelStringElem<'a> {}

impl<'a> PartialEq for ParcelStringElem<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl<'a> Hash for ParcelStringElem<'a> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let as_string = self.to_string();
        state.write(as_string.as_bytes());
    }
}

impl<'a> Display for ParcelStringElem<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ParcelStringElem::Long(v) => write!(f, "LONG{}", v),
            ParcelStringElem::Int(v) => write!(f, "_INT{}", v),
            ParcelStringElem::Short(v) => write!(f, "SHRT{}", v),
            ParcelStringElem::Byte(v) => write!(f, "BYTE{}", v),
            ParcelStringElem::Bool(v) => write!(f, "BOOL{}", v),
            ParcelStringElem::Double(v) => write!(f, "DUBL{}", v),
            ParcelStringElem::Float(v) => write!(f, "_FLT{}", v),
            ParcelStringElem::Binder => write!(f, "BIND"),
            ParcelStringElem::Null => write!(f, "NULL"),
            ParcelStringElem::WriteFd(v) => write!(f, "WRFD{}", escape_string(v)),
            ParcelStringElem::ReadFd(v) => write!(f, "RDFD{}", escape_string(v)),
            ParcelStringElem::String(v) => write!(f, "_STR{}", escape_string(v)),
            ParcelStringElem::RawByteArray(v) => write!(f, "BARR{}", bytes_to_hex(v)),
            ParcelStringElem::HexByteArray(v) => write!(f, "BARR{}", v),
            ParcelStringElem::Message(what, arg1, arg2, data) => {
                write!(f, "_MSG{what}%{arg1}%{arg2}")?;
                if let Some(v) = data {
                    write!(f, "%")?;
                    let last = v.len() - 1;
                    for (i, (key, value)) in v.iter().enumerate() {
                        write!(f, "{}={}", escape_string(key), value)?;
                        if i < last {
                            write!(f, ":")?;
                        }
                    }
                }
                Ok(())
            }
            ParcelStringElem::Bundle(v) => {
                write!(f, "BUND")?;
                if v.len() > 0 {
                    let last = v.len() - 1;
                    for (i, (key, value)) in v.iter().enumerate() {
                        write!(f, "{}={}", escape_string(key), value)?;
                        if i < last {
                            write!(f, ":")?;
                        }
                    }
                }
                Ok(())
            }
            ParcelStringElem::Map(v) => {
                write!(f, "_MAP")?;
                if v.len() > 0 {
                    let last = v.len() - 1;
                    for (i, (key, value)) in v.iter().enumerate() {
                        write!(f, "{}={}", key, value)?;
                        if i < last {
                            write!(f, ":")?;
                        }
                    }
                }
                Ok(())
            }
            ParcelStringElem::List(v) => {
                write!(f, "_LST")?;
                if v.len() > 0 {
                    let last = v.len() - 1;
                    for (i, e) in v.iter().enumerate() {
                        write!(f, "{}", e)?;
                        if i < last {
                            write!(f, ":")?;
                        }
                    }
                }
                Ok(())
            }
            ParcelStringElem::Complex(v) => {
                write!(f, "_CMB")?;
                if v.len() > 0 {
                    let last = v.len() - 1;
                    for (i, e) in v.iter().enumerate() {
                        write!(f, "{}", e)?;
                        if i < last {
                            write!(f, "|")?;
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_escape_string() {
        let s = "custom,escapes=wow:suchgreat|asdf>cool";
        assert_eq!(
            escape_string(s),
            "custom\\,escapes\\=wow\\:suchgreat\\|asdf\\>cool"
        );
    }

    #[test]
    fn test_parcel_string_builder() {
        let mut into = String::with_capacity(128);
        let mut builder = ParcelString::default();
        macro_rules! simple_test {
            ($func:ident, $val:expr, $expected:expr) => {{
                builder.elems.clear();
                into.clear();
                builder.$func($val);
                builder.build_to(&mut into);
                assert_eq!(into, $expected, "{} failed", $expected);
            }};
        }

        simple_test!(write_int, i32::MIN, format!("_INT{}", i32::MIN));
        simple_test!(write_int, i32::MAX, format!("_INT{}", i32::MAX));

        simple_test!(write_bool, true, "BOOLtrue");
        simple_test!(write_bool, false, "BOOLfalse");
    }
}
