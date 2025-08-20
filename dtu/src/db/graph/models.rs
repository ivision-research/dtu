use crate::utils::ClassName;
use smalisa::AccessFlag;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(PartialEq, Eq, Debug, PartialOrd, Ord))]
pub struct ClassMeta {
    pub name: ClassName,
    #[serde(skip, default)]
    pub access_flags: AccessFlag,
    pub source: String,
}

impl ClassMeta {
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
pub struct ClassCallPath {
    /// The class the call originates in
    pub class: ClassName,
    /// The path of methods that ends up at the target call
    pub path: Vec<MethodMeta>,
}

#[derive(PartialEq, Eq, Hash, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(Debug, PartialOrd, Ord))]
pub struct ClassSourceCallPath {
    /// The class the call originates in
    pub class: ClassName,
    /// The source containing the originating class
    pub source: String,
    /// The path of methods that ends up at the target call
    pub path: Vec<MethodMeta>,
}

#[derive(PartialEq, Eq, Hash, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(Debug, PartialOrd, Ord))]
pub struct MethodMeta {
    pub class: ClassName,
    pub ret: Option<String>,
    pub name: String,
    pub signature: String,
    #[serde(skip, default)]
    pub access_flags: AccessFlag,
}

impl MethodMeta {
    pub fn as_smali(&self) -> String {
        let mut smali = format!(
            "{}->{}({})",
            self.class.get_smali_name(),
            self.name,
            self.signature
        );
        if let Some(ret) = &self.ret {
            smali.push_str(ret);
        }
        smali
    }

    pub fn from_smali(smali: &str) -> super::Result<Self> {
        let (raw_class, rem) = match smali.split_once("->") {
            None => {
                return Err(super::Error::Generic(format!(
                    "invalid smali method meta (no ->): {}",
                    smali
                )))
            }
            Some(v) => v,
        };

        if rem.len() == 0 {
            return Err(super::Error::Generic(format!(
                "invalid smali method meta (nothing after the ->) : {}",
                smali
            )));
        }

        let (raw_method, raw_sig) = match rem.split_once('(') {
            None => {
                return Err(super::Error::Generic(format!(
                    "invalid smali method meta (no signature): {}",
                    smali
                )))
            }
            Some(v) => v,
        };

        let class = ClassName::from(raw_class);
        return Ok(Self {
            class,
            name: raw_method.into(),
            signature: raw_sig.trim_end_matches(')').into(),
            ret: None,
            access_flags: AccessFlag::UNSET,
        });
    }
}

/// Contains everything needed to search for a method call
pub struct MethodCallSearch<'a> {
    /// The target method name
    pub target_method: &'a str,
    /// Specifies the signature of the target method
    pub target_method_sig: &'a str,

    /// Specifies the class doing the calling
    pub src_class: Option<&'a ClassName>,
    /// Specifies the method name doing the calling
    pub src_method_name: Option<&'a str>,
    /// Specifies the calling method's signature
    pub src_method_sig: Option<&'a str>,

    /// Specifies the class owning the target method
    pub target_class: Option<&'a ClassName>,

    /// Source is where the call was discovered
    pub source: Option<&'a str>,
}

pub struct MethodSearch<'a> {
    pub name: &'a str,
    pub class: Option<&'a ClassName>,
    pub signature: Option<&'a str>,
    pub source: Option<&'a str>,
}

impl<'a> MethodSearch<'a> {
    pub fn new<C: AsRef<ClassName> + 'a>(
        name: &'a str,
        class: Option<&'a C>,
        signature: Option<&'a str>,
        source: Option<&'a str>,
    ) -> Self {
        Self {
            name,
            class: class.map(|it| it.as_ref()),
            signature,
            source,
        }
    }
}
