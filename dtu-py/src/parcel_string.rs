use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use dtu::utils::bytes_to_hex;
use pyo3::prelude::*;

#[pyclass(name = "ParcelValue")]
#[derive(Clone)]
pub enum ParcelValue {
    Long {
        long: i64,
    },
    Int {
        int: i32,
    },
    Short {
        short: i16,
    },
    Byte {
        byte: u8,
    },
    Bool {
        bool: bool,
    },
    Double {
        double: f64,
    },
    Float {
        float: f32,
    },
    Binder(),
    Null(),
    WriteFd {
        write_fd: String,
    },
    ReadFd {
        read_fd: String,
    },
    String {
        string: String,
    },
    RawByteArray {
        raw_byte_array: Vec<u8>,
    },
    HexByteArray {
        hex_byte_array: String,
    },
    Bundle {
        bundle: HashMap<String, ParcelValue>,
    },
    Map {
        map: HashMap<ParcelValue, ParcelValue>,
    },
    List {
        list: Vec<ParcelValue>,
    },
    Complex {
        complex: Vec<ParcelValue>,
    },
}

#[pymethods]
impl ParcelValue {
    fn __str(&self) -> String {
        self.__repr__()
    }
    fn __repr__(&self) -> String {
        match self {
            Self::Long { long: v } => format!("ParcelValue.Long({v})"),
            Self::Int { int: v } => format!("ParcelValue.Int({v})"),
            Self::Short { short: v } => format!("ParcelValue.Short({v})"),
            Self::Byte { byte: v } => format!("ParcelValue.Byte({v})"),
            Self::Bool { bool: v } => format!("ParcelValue.Bool({v})"),
            Self::Double { double: v } => format!("ParcelValue.Double({v})"),
            Self::Float { float: v } => format!("ParcelValue.Float({v})"),
            Self::WriteFd { write_fd: v } => format!("ParcelValue.WriteFd({v:?})"),
            Self::ReadFd { read_fd: v } => format!("ParcelValue.ReadFd({v:?})"),
            Self::String { string: v } => format!("ParcelValue.String({v:?})"),
            Self::Binder() => String::from("ParcelValue.Binder()"),
            Self::Null() => String::from("ParcelValue.Null()"),
            Self::RawByteArray { raw_byte_array: v } => {
                let mut s = String::with_capacity(3 + v.len() * 4);
                s.push_str("b\"");
                for b in v {
                    s.push_str(&format!("\\x{b:02x}"));
                }
                s.push('"');
                format!("ParcelValue.RawByteArray({s})")
            }
            Self::HexByteArray { hex_byte_array: v } => format!("ParcelValue.HexByteArray({v})"),
            Self::Map { map } => format!(
                "ParcelValue.Map({{{}}})",
                map.iter()
                    .map(|(k, v)| format!("{}: {}", k.__repr__(), v.__repr__()))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),

            Self::Bundle { bundle } => format!(
                "ParcelValue.Bundle({{{}}})",
                bundle
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.__repr__()))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Self::List { list } => format!(
                "ParcelValue.List([{}])",
                list.iter()
                    .map(|it| it.__repr__())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Self::Complex { complex } => format!(
                "ParcelValue.Complex([{}])",
                complex
                    .iter()
                    .map(|it| it.__repr__())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
        }
    }
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
            ParcelValue::Long { long: v } => write!(f, "LONG{}", v),
            ParcelValue::Int { int: v } => write!(f, "_INT{}", v),
            ParcelValue::Short { short: v } => write!(f, "SHRT{}", v),
            ParcelValue::Byte { byte: v } => write!(f, "BYTE{}", v),
            ParcelValue::Bool { bool: v } => write!(f, "BOOL{}", v),
            ParcelValue::Double { double: v } => write!(f, "DUBL{}", v),
            ParcelValue::Float { float: v } => write!(f, "_FLT{}", v),
            ParcelValue::Binder() => write!(f, "BIND"),
            ParcelValue::Null() => write!(f, "NULL"),
            ParcelValue::WriteFd { write_fd: v } => write!(f, "WRFD{}", escape_string(v)),
            ParcelValue::ReadFd { read_fd: v } => write!(f, "RDFD{}", escape_string(v)),
            ParcelValue::String { string: v } => write!(f, "_STR{}", escape_string(v)),
            ParcelValue::RawByteArray { raw_byte_array: v } => write!(f, "BARR{}", bytes_to_hex(v)),
            ParcelValue::HexByteArray { hex_byte_array: v } => write!(f, "BARR{}", v),
            ParcelValue::Bundle { bundle: v } => {
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
            ParcelValue::Map { map: v } => {
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
            ParcelValue::List { list: v } => {
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
            ParcelValue::Complex { complex: v } => {
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
