use smalisa::{parse_method_args, Type};
use std::vec::IntoIter;

pub struct SmaliMethodSignatureIterator<'a> {
    parsed: IntoIter<Type<'a>>,
}

impl<'a> SmaliMethodSignatureIterator<'a> {
    pub fn new(signature: &'a str) -> Result<Self, &'a str> {
        let parsed = parse_method_args(signature)?.into_iter();
        Ok(Self { parsed })
    }
}

impl<'a> Iterator for SmaliMethodSignatureIterator<'a> {
    type Item = Type<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.parsed.next()
    }
}
