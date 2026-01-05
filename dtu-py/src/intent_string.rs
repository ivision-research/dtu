use std::collections::HashMap;

use crate::parcel_string::{escape_string, ParcelValue};

pub(crate) fn build_intent_string(elems: &HashMap<String, ParcelValue>) -> String {
    let mut into = String::new();
    let last = elems.len() - 1;
    for (i, (key, value)) in elems.iter().enumerate() {
        into.push_str(&format!("{}>{}", escape_string(key), value));
        if i < last {
            into.push(',');
        }
    }
    into
}
