use anyhow::bail;
use dtu::app::{IntentString, ParcelString, ParcelStringElem};
use std::borrow::Cow;
use std::collections::HashMap;
use std::slice::Iter;

pub fn parse_intent_string(args: &[String]) -> anyhow::Result<IntentString> {
    let mut intent_string = IntentString::default();

    let mut iter = args.iter();

    loop {
        let key = match iter.next() {
            Some(v) => v.clone(),
            None => break,
        };

        match parse_single(&mut iter)? {
            Some(v) => {
                intent_string.push(key, v);
            }
            None => bail!("no value for key {}", key),
        }
    }

    Ok(intent_string)
}

pub fn parse_parcel_string(args: &[String]) -> anyhow::Result<ParcelString> {
    let mut parcel_string = ParcelString::default();

    let mut iter = args.iter();

    loop {
        match parse_single(&mut iter)? {
            Some(v) => {
                parcel_string.push(v);
            }
            None => break,
        }
    }

    Ok(parcel_string)
}

fn parse_single(iter: &mut Iter<String>) -> anyhow::Result<Option<ParcelStringElem<'static>>> {
    let ident = match iter.next() {
        None => return Ok(None),
        Some(v) => v.as_str(),
    };

    match ident {
        "end" => {
            return Ok(None);
        }
        "bind" => {
            return Ok(Some(ParcelStringElem::Binder));
        }
        "null" => {
            return Ok(Some(ParcelStringElem::Null));
        }
        "map" => {
            let mut map = HashMap::new();
            loop {
                let key = match parse_single(iter)? {
                    Some(v) => v,
                    None => break,
                };
                let value = match parse_single(iter)? {
                    None => bail!("expected a value for key {}", key),
                    Some(v) => v,
                };
                map.insert(key, value);
            }
            return Ok(Some(ParcelStringElem::Map(map)));
        }
        "bund" => {
            let mut map = HashMap::new();
            loop {
                let key = match iter.next() {
                    None => bail!("no end of bundle found"),
                    Some(v) => v,
                };
                if key == "end" {
                    break;
                }
                let value = match parse_single(iter)? {
                    None => bail!("expected a value for bundle key {}", key),
                    Some(v) => v,
                };
                map.insert(key.clone(), value);
            }
            return Ok(Some(ParcelStringElem::Bundle(map)));
        }
        "list" => {
            let mut elems = Vec::new();
            loop {
                let it = match parse_single(iter)? {
                    None => break,
                    Some(v) => v,
                };
                elems.push(it);
            }
            return Ok(Some(ParcelStringElem::List(elems)));
        }
        _ => {}
    }

    let next = match iter.next() {
        None => bail!("{} requires an argument", ident),
        Some(v) => v.as_str(),
    };

    match ident {
        "barr" => {
            if next.len() % 2 != 0 {
                bail!("invalid hex string for barr: {}", next);
            }
            let lower = next.to_lowercase();
            for b in lower.bytes() {
                match b {
                    b'0'..=b'9' => {}
                    b'a'..=b'f' => {}
                    _ => bail!(
                        "invalid hex string for barr: {}, {} is not a hex char",
                        next,
                        b as char
                    ),
                }
            }
            return Ok(Some(ParcelStringElem::HexByteArray(Cow::Owned(lower))));
        }
        "str" => {
            return Ok(Some(ParcelStringElem::String(Cow::Owned(String::from(
                next,
            )))));
        }
        "f64" => {
            return Ok(Some(ParcelStringElem::Double(next.parse::<f64>()?)));
        }
        "f32" => {
            return Ok(Some(ParcelStringElem::Float(next.parse::<f32>()?)));
        }

        "i64" => {
            return Ok(Some(ParcelStringElem::Long(next.parse::<i64>()?)));
        }
        "i32" => {
            return Ok(Some(ParcelStringElem::Int(next.parse::<i32>()?)));
        }
        "i16" => {
            return Ok(Some(ParcelStringElem::Short(next.parse::<i16>()?)));
        }

        "u8" => {
            return Ok(Some(ParcelStringElem::Byte(next.parse::<u8>()?)));
        }
        "z" => {
            return Ok(Some(ParcelStringElem::Bool(next.parse::<bool>()?)));
        }

        "wfd" => {
            return Ok(Some(ParcelStringElem::WriteFd(Cow::Owned(String::from(
                next,
            )))));
        }
        "rfd" => {
            return Ok(Some(ParcelStringElem::ReadFd(Cow::Owned(String::from(
                next,
            )))));
        }

        _ => bail!("unknown type {}", ident),
    }
}

#[cfg(test)]
mod test {
    use crate::parsers::parse_parcel_string;

    #[test]
    fn test_parse_parcel() {
        let pcl = &[
            String::from("i64"),
            String::from("-100"),
            String::from("i32"),
            String::from("10"),
            String::from("str"),
            String::from("hello world"),
            String::from("z"),
            String::from("false"),
        ];
        let parsed = parse_parcel_string(pcl).expect("parsing failed").build();
        assert_eq!(parsed, "LONG-100,_INT10,_STRhello world,BOOLfalse")
    }
}
