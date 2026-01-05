use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use dtu::utils::bytes_to_hex;
use pyo3::prelude::*;

#[pyclass(name = "ParcelValue")]
#[derive(Clone)]
pub enum ParcelValue {
    Long(i64),
    Int(i32),
    Short(i16),
    Byte(u8),
    Bool(bool),
    Double(f64),
    Float(f32),
    Binder(),
    Null(),
    WriteFd(String),
    ReadFd(String),
    String(String),
    RawByteArray(Vec<u8>),
    HexByteArray(String),
    Bundle(HashMap<String, ParcelValue>),
    Map(HashMap<ParcelValue, ParcelValue>),
    List(Vec<ParcelValue>),
    Complex(Vec<ParcelValue>),
}

impl Eq for ParcelValue {}

impl PartialEq for ParcelValue {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl Hash for ParcelValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let as_string = self.to_string();
        state.write(as_string.as_bytes());
    }
}

impl Display for ParcelValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ParcelValue::Long(v) => write!(f, "LONG{}", v),
            ParcelValue::Int(v) => write!(f, "_INT{}", v),
            ParcelValue::Short(v) => write!(f, "SHRT{}", v),
            ParcelValue::Byte(v) => write!(f, "BYTE{}", v),
            ParcelValue::Bool(v) => write!(f, "BOOL{}", v),
            ParcelValue::Double(v) => write!(f, "DUBL{}", v),
            ParcelValue::Float(v) => write!(f, "_FLT{}", v),
            ParcelValue::Binder() => write!(f, "BIND"),
            ParcelValue::Null() => write!(f, "NULL"),
            ParcelValue::WriteFd(v) => write!(f, "WRFD{}", escape_string(v)),
            ParcelValue::ReadFd(v) => write!(f, "RDFD{}", escape_string(v)),
            ParcelValue::String(v) => write!(f, "_STR{}", escape_string(v)),
            ParcelValue::RawByteArray(v) => write!(f, "BARR{}", bytes_to_hex(v)),
            ParcelValue::HexByteArray(v) => write!(f, "BARR{}", v),
            ParcelValue::Bundle(v) => {
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
            ParcelValue::Map(v) => {
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
            ParcelValue::List(v) => {
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
            ParcelValue::Complex(v) => {
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

pub(crate) fn build_parcel_string(elems: &Vec<ParcelValue>) -> String {
    let mut into = String::new();
    let last = elems.len() - 1;
    for (i, e) in elems.iter().enumerate() {
        let as_str = e.to_string();
        into.push_str(&as_str);
        if i < last {
            into.push(',');
        }
    }
    into
}

pub(crate) fn escape_string(s: &str) -> String {
    s.escape_default()
        .to_string()
        .replace("=", "\\=")
        .replace(",", "\\,")
        .replace(":", "\\:")
        .replace("|", "\\|")
        .replace(">", "\\>")
}
