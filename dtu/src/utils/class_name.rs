use serde::{Deserialize, Serialize, Serializer};
use std::borrow::Cow;
use std::convert::Infallible;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[cfg(feature = "sql")]
use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    serialize::{Output, ToSql},
    sql_types::Text,
    AsExpression,
};

// This file and the ClassName type has gone through a bunch of internal changes and at this point
// it really would benefit from a redesign similar to the way smali handles things. The ClassName
// type has resulted in so many completely unnecessary string copies :(

/// Single type to represent both smali and java class names
///
/// This class always appears as the smali format of the class, but it allows operations as if it
/// were the Java format.
#[derive(Eq, Debug, Clone, Ord, PartialOrd)]
#[cfg_attr(feature = "sql", derive(FromSqlRow, AsExpression))]
#[cfg_attr(feature = "sql", diesel(sql_type = Text))]
pub struct ClassName {
    name: String,
}

impl From<&smalisa::SmaliClassName> for ClassName {
    fn from(value: &smalisa::SmaliClassName) -> Self {
        Self::new(value.as_str().to_string())
    }
}

impl From<&String> for ClassName {
    fn from(value: &String) -> Self {
        Self::new(value.into())
    }
}

impl From<String> for ClassName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ClassName {
    fn from(value: &str) -> Self {
        Self::new(value.into())
    }
}

impl AsRef<str> for ClassName {
    fn as_ref(&self) -> &str {
        self.name.as_str()
    }
}

impl AsRef<ClassName> for ClassName {
    fn as_ref(&self) -> &ClassName {
        self
    }
}

impl ClassName {
    pub fn new(name: String) -> Self {
        let name = if class_is_smali(&name) {
            name
        } else {
            java_name_to_smali(&name)
        };
        Self { name }
    }

    /// Get a ClassName from a Manifest entry of the form pkg/class
    /// after it's been split.
    ///
    /// There are two scenarios for Manifest entries like this:
    ///
    /// - com.foo.bar/foo.bar.Baz
    /// - com.foo.bar/.Baz
    ///
    /// These entries would represent classes named foo.bar.Baz and
    /// com.foo.bar.Baz respectively.
    pub fn from_split_manifest(pkg: &str, name: &str) -> Self {
        if name.starts_with('.') {
            Self::new(format!("{}{}", pkg, name))
        } else {
            Self::new(String::from(name))
        }
    }

    /// Checks to see if the class has a package
    pub fn has_pkg(&self) -> bool {
        if self.is_smali() {
            self.name.contains('/')
        } else {
            // We're sometimes dealing with names like:
            //
            // .Class
            //
            // when dealing with Android manifest files and the like
            self.name.trim_start_matches('.').contains('.')
        }
    }

    /// Creates a new ClassName with the same simple name but a different
    /// package
    pub fn with_new_package(&self, pkg: &str) -> ClassName {
        let simple = self.get_simple_class_name();
        let is_smali_pkg = pkg.contains('/');
        let new_name = if is_smali_pkg {
            format!("L{}/{};", pkg.trim_end_matches('/'), simple)
        } else {
            format!("{}.{}", pkg.trim_end_matches('.'), simple)
        };

        ClassName::new(new_name)
    }

    pub fn as_str(&self) -> &str {
        self.name.as_str()
    }

    /// Change the simple name of the class
    pub fn with_new_simple_class_name(&self, name: &str) -> ClassName {
        if self.is_smali() {
            let prefix = self.name.rsplit_once('/').map(|it| it.0);
            match prefix {
                None => Self::new(format!("L{name};")),
                Some(v) => Self::new(format!("{v}/{name};")),
            }
        } else {
            let mut base = self.pkg_as_java().to_string();
            base.push('.');
            base.push_str(name);
            Self::new(base)
        }
    }

    /// Get the simple class name
    pub fn get_simple_class_name(&self) -> &str {
        if self.is_java() {
            let idx = match self.name.rfind('.') {
                Some(i) => i + 1,
                None => 0,
            };
            let (_, name) = self.name.as_str().split_at(idx);
            name
        } else {
            let idx = match self.name.rfind('/') {
                Some(i) => i + 1,
                // 1 to skip the L
                None => 1,
            };
            let (_, name) = self.name.as_str().split_at(idx);
            name.trim_end_matches(';')
        }
    }

    /// Gets the class package as a Java dotted package
    pub fn pkg_as_java(&self) -> Cow<'_, str> {
        let name = self.get_java_name();

        let start = name.rfind('.').unwrap_or(0);

        if start == 0 {
            return Cow::Borrowed("");
        }

        match name {
            Cow::Owned(owned) => {
                let (pkg, _) = owned.split_at(start);
                Cow::Owned(String::from(pkg))
            }
            Cow::Borrowed(borrowed) => {
                let (pkg, _) = borrowed.split_at(start);
                Cow::Borrowed(pkg)
            }
        }
    }

    #[inline]
    pub fn is_smali(&self) -> bool {
        true
    }

    #[inline]
    pub fn is_java(&self) -> bool {
        false
    }

    pub fn get_java_name(&self) -> Cow<'_, str> {
        if self.is_java() {
            Cow::Borrowed(self.name.as_str())
        } else {
            Cow::Owned(smali_name_to_java(&self.name))
        }
    }

    pub fn get_smali_name(&self) -> Cow<'_, str> {
        if self.is_smali() {
            Cow::Borrowed(self.name.as_str())
        } else {
            Cow::Owned(java_name_to_smali(&self.name))
        }
    }
}

impl Serialize for ClassName {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let smali_name = self.get_smali_name();
        smali_name.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ClassName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(ClassName::new(String::deserialize(deserializer)?))
    }
}

impl FromStr for ClassName {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ClassName::from(s))
    }
}

fn class_is_smali(s: &str) -> bool {
    s.starts_with('L') && s.ends_with(';')
}

fn java_name_to_smali(name: &str) -> String {
    let mut new_name = String::with_capacity(name.len() + 2);
    new_name.push('L');
    for c in name.chars() {
        if c == '.' {
            new_name.push('/');
        } else {
            new_name.push(c);
        }
    }
    new_name.push(';');
    new_name
}

fn smali_name_to_java(name: &str) -> String {
    let mut new_name = String::with_capacity(name.len() - 2);
    for c in name.chars().skip(1).take(name.len() - 2) {
        if c == '/' {
            new_name.push('.');
        } else {
            new_name.push(c);
        }
    }
    new_name
}

impl Hash for ClassName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.get_smali_name().as_bytes())
    }
}

impl<T: AsRef<str> + ?Sized> PartialEq<T> for ClassName {
    fn eq(&self, other: &T) -> bool {
        let as_str = other.as_ref();
        if class_is_smali(as_str) {
            self.get_smali_name() == as_str
        } else {
            self.get_java_name() == as_str
        }
    }
}

impl Display for ClassName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

#[cfg(feature = "sql")]
impl<DB> FromSql<Text, DB> for ClassName
where
    DB: Backend,
    String: FromSql<Text, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let class_name = String::from_sql(bytes)?;
        Ok(Self::new(class_name))
    }
}

#[cfg(feature = "sql")]
impl<DB> ToSql<Text, DB> for ClassName
where
    DB: Backend,
    String: ToSql<Text, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> diesel::serialize::Result {
        self.name.to_sql(out)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_simple_class_name() {
        let class = ClassName::from("com.test.Class$Stub");
        assert_eq!(class.get_simple_class_name(), "Class$Stub");
        let class = ClassName::from("Lcom/test/Class$Stub;");
        assert_eq!(class.get_simple_class_name(), "Class$Stub");
        let class = ClassName::from("Class$Stub");
        assert_eq!(class.get_simple_class_name(), "Class$Stub");
        let class = ClassName::from("LClass$Stub;");
        assert_eq!(class.get_simple_class_name(), "Class$Stub");
    }

    #[test]
    fn test_with_new_simple_class_name() {
        let original = ClassName::from("com.test.Class");
        let new = original.with_new_simple_class_name("Class$Stub");
        assert_eq!(new.get_java_name().as_ref(), "com.test.Class$Stub");
    }

    #[test]
    fn test_has_package() {
        let has = ClassName::from("com.test.Class");
        assert!(has.has_pkg());
        let has = ClassName::from("Lcom/test/Class;");
        assert!(has.has_pkg());
        let doesnt = ClassName::from("Class");
        assert!(!doesnt.has_pkg());
        let doesnt = ClassName::from("LClass;");
        assert!(!doesnt.has_pkg());
    }

    #[test]
    fn test_class_name_simple_name() {
        let java_name = ClassName::from("java.lang.String");
        assert_eq!(java_name.get_simple_class_name(), "String");
        let smali_name = ClassName::from("Ljava/lang/String;");
        assert_eq!(smali_name.get_simple_class_name(), "String");
    }

    #[test]
    fn test_class_name_pkg() {
        let java_name = ClassName::from("java.lang.String");
        let smali_name = ClassName::from("Ljava/lang/String;");
        let java_name = java_name.pkg_as_java();
        assert_eq!(
            java_name.as_str(),
            "java.lang",
            "bad java name for java class"
        );
        let java_name = smali_name.pkg_as_java();
        assert_eq!(
            java_name.as_str(),
            "java.lang",
            "bad java name for smali class"
        );
    }

    #[test]
    fn test_class_name_eq() {
        let java_name = ClassName::from("java.lang.String");
        let smali_name = ClassName::from("Ljava/lang/String;");
        assert_eq!(java_name, smali_name);
        assert_eq(java_name, "Ljava/lang/String;");
        assert_eq(java_name, "java.lang.String");
        assert_eq(smali_name, "Ljava/lang/String;");
        assert_eq(smali_name, "java.lang.String");
    }

    #[test]
    fn test_class_name_conversions() {
        let cn = ClassName::from("java.lang.String");
        assert_eq!(cn.get_java_name().as_ref(), "java.lang.String");
        assert_eq!(cn.get_smali_name().as_ref(), "Ljava/lang/String;");
        let cn = ClassName::from("Ljava/lang/String;");
        assert_eq!(cn.get_java_name().as_ref(), "java.lang.String");
        assert_eq!(cn.get_smali_name().as_ref(), "Ljava/lang/String;");
    }

    #[test]
    fn test_smali_name_to_java() {
        let smali_name = "Ljava/lang/String;";
        assert_eq!(smali_name_to_java(smali_name), "java.lang.String");
        let smali_name = "La;";
        assert_eq!(smali_name_to_java(smali_name), "a");
    }

    #[test]
    fn test_java_name_to_smali() {
        let java_name = "java.lang.String";
        assert_eq!(java_name_to_smali(java_name), "Ljava/lang/String;");
        let java_name = "a";
        assert_eq!(java_name_to_smali(java_name), "La;");
    }

    #[test]
    fn test_class_name_display() {
        let disp = "Ljava/lang/String;";
        let java_name = ClassName::from("java.lang.String");
        let smali_name = ClassName::from(disp);
        assert_eq!(java_name.to_string().as_str(), disp, "java class display");
        assert_eq!(smali_name.to_string().as_str(), disp, "smali class display");
    }
}
