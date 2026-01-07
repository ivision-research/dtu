use std::fmt::Display;

use crate::utils::ClassName;
use smalisa::AccessFlag;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(PartialEq, Eq, Debug, PartialOrd, Ord))]
pub struct ClassSpec {
    pub name: ClassName,
    #[serde(skip, default)]
    pub access_flags: AccessFlag,
    pub source: String,
}

impl ClassSpec {
    pub fn is_public(&self) -> bool {
        self.access_flags.is_public()
    }

    pub fn is_not_abstract(&self) -> bool {
        let bad_flags = AccessFlag::ABSTRACT | AccessFlag::INTERFACE;
        return !self.access_flags.intersects(bad_flags);
    }
}

#[derive(PartialEq, Eq, Hash, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(Debug, PartialOrd, Ord))]
pub struct MethodCallPath {
    /// The path of methods that ends up at the target call
    pub path: Vec<MethodSpec>,
}

impl MethodCallPath {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }
    #[inline]
    pub fn is_not_empty(&self) -> bool {
        !self.is_empty()
    }

    pub fn get_src_method(&self) -> Option<&MethodSpec> {
        self.path.first()
    }

    pub fn get_dst_method(&self) -> Option<&MethodSpec> {
        self.path.last()
    }

    pub fn get_src_class(&self) -> Option<&ClassName> {
        self.get_src_method().map(|it| &it.class)
    }

    pub fn get_dst_class(&self) -> Option<&ClassName> {
        self.get_dst_method().map(|it| &it.class)
    }

    pub fn get_source(&self) -> Option<&str> {
        self.get_src_method().map(|it| it.source.as_str())
    }

    pub fn must_get_src_method(&self) -> &MethodSpec {
        self.get_src_method().expect("get_src_method")
    }

    pub fn must_get_dst_method(&self) -> &MethodSpec {
        self.get_dst_method().expect("get_dst_method")
    }

    pub fn must_get_src_class(&self) -> &ClassName {
        self.get_src_class().expect("get_src_class")
    }

    pub fn must_get_dst_class(&self) -> &ClassName {
        self.get_dst_class().expect("get_dst_class")
    }

    pub fn must_get_source(&self) -> &str {
        self.get_source().expect("get_source")
    }
}

impl From<Vec<MethodSpec>> for MethodCallPath {
    fn from(path: Vec<MethodSpec>) -> Self {
        Self { path }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(Debug, PartialOrd, Ord))]
pub struct MethodSpec {
    pub class: ClassName,
    pub name: String,
    pub signature: String,
    pub ret: String,
    pub source: String,
}

impl Display for MethodSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}->{}({}){}",
            self.class.get_smali_name(),
            self.name,
            self.signature,
            self.ret
        )
    }
}

impl MethodSpec {
    pub fn as_smali(&self) -> String {
        self.to_string()
    }
}

pub enum MethodSearchParams<'a> {
    /// Search for the method by name only: *bar*
    ByName { name: &'a str },

    /// Search for the method by class only: Lfoo;->*
    ByClass { class: &'a ClassName },

    /// Search for the method by name and signature: *bar(IZ)
    ByNameAndSignature { name: &'a str, signature: &'a str },

    /// Search for the method by class and name
    ByClassAndName { class: &'a ClassName, name: &'a str },

    /// Search for a fully specified method: Lfoo;->bar(IZ)
    ByFullSpec {
        class: &'a ClassName,
        name: &'a str,
        signature: &'a str,
    },
}

impl<'a> MethodSearchParams<'a> {
    pub fn new(
        name: Option<&'a str>,
        class: Option<&'a ClassName>,
        signature: Option<&'a str>,
    ) -> Result<Self, &'static str> {
        Ok(match name {
            Some(name) => match class {
                Some(class) => match signature {
                    Some(signature) => Self::ByFullSpec {
                        class,
                        name,
                        signature,
                    },
                    None => Self::ByClassAndName { class, name },
                },
                None => match signature {
                    Some(signature) => Self::ByNameAndSignature { name, signature },
                    None => Self::ByName { name },
                },
            },

            None => match class {
                Some(class) => match signature {
                    None => Self::ByClass { class },
                    Some(_) => return Err("class and signature only currently unsupported"),
                },
                None => return Err("need name or class"),
            },
        })
    }
}

pub struct ClassSearch<'a> {
    pub class: &'a ClassName,
    pub source: Option<&'a str>,
}

impl<'a> From<&'a ClassName> for ClassSearch<'a> {
    fn from(value: &'a ClassName) -> Self {
        Self::new(value, None)
    }
}

impl<'a> ClassSearch<'a> {
    #[inline]
    pub fn with_source(mut self, source: &'a str) -> Self {
        self.source = Some(source);
        self
    }
    pub fn new(class: &'a ClassName, source: Option<&'a str>) -> Self {
        Self { class, source }
    }
}

/// Specify a method to search for
pub struct MethodSearch<'a> {
    pub param: MethodSearchParams<'a>,
    pub source: Option<&'a str>,
}

impl<'a> From<MethodSearchParams<'a>> for MethodSearch<'a> {
    fn from(value: MethodSearchParams<'a>) -> Self {
        Self::new(value, None)
    }
}

impl<'a> MethodSearch<'a> {
    #[inline]
    pub fn with_source(mut self, source: &'a str) -> Self {
        self.source = Some(source);
        self
    }

    pub fn new(param: MethodSearchParams<'a>, source: Option<&'a str>) -> Self {
        Self { param, source }
    }

    pub fn new_from_opts(
        class: Option<&'a ClassName>,
        name: Option<&'a str>,
        signature: Option<&'a str>,
        source: Option<&'a str>,
    ) -> Result<Self, &'static str> {
        let param = MethodSearchParams::new(name, class, signature)?;
        Ok(Self { param, source })
    }
}
