use super::{escape_string, ParcelStringElem};

#[cfg_attr(test, derive(Debug))]
pub struct IntentString<'a> {
    values: Vec<(String, ParcelStringElem<'a>)>,
}

impl<'a> IntentString<'a> {
    pub fn build(&self) -> String {
        let mut into = String::new();
        self.build_to(&mut into);
        into
    }

    pub fn build_to(&self, into: &mut String) {
        let last = self.values.len() - 1;
        for (i, (key, value)) in self.values.iter().enumerate() {
            into.push_str(&format!("{}>{}", escape_string(key), value));
            if i < last {
                into.push(',');
            }
        }
    }

    pub fn push(&mut self, key: String, value: ParcelStringElem<'a>) -> &mut Self {
        self.values.push((key, value));
        self
    }
}

impl<'a> Default for IntentString<'a> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}
