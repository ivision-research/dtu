use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::path::{Path, PathBuf};

use crate::{DEVICE_PATH_SEP_CHAR, REPLACED_DEVICE_PATH_SEP_CHAR};
use serde::de::Visitor;
use serde::{Deserialize, Serialize};

#[cfg(feature = "sql")]
use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    serialize::{Output, ToSql},
    sql_types::Text,
    AsExpression,
};

use super::{replace_char, unreplace_char, OS_PATH_SEP_CHAR};

/// DevicePaths are used throughout this library as a way to represent paths
/// as they were on the actual Android device.
///
/// This type contains two different views of the path:
///
/// The raw view -> /system/priv-app/Test.apk
/// A squashed view -> %system%priv-app%Test.apk
///
/// We primarily use this in two places:
///
/// 1. When pulling from the device
/// 2. For identifying APKs
///
/// This type just wraps some helpful functionality to make it simpler to work
/// with. For example, when this type is Displayed, the raw path is show.
/// However when this type is viewed as a `&Path` (via `AsRef<Path>`) the
/// squashed path is used.
///
/// Note that for with `%` in their name, squashed views will escape them as `\%`.
#[derive(Clone, Eq)]
#[cfg_attr(feature = "sql", derive(FromSqlRow, AsExpression))]
#[cfg_attr(feature = "sql", diesel(sql_type = Text))]
pub struct DevicePath {
    raw_path: String,
    squashed_path: String,
}

impl Serialize for DevicePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.raw_path)
    }
}

impl<'de> Deserialize<'de> for DevicePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = DevicePath;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a device path")
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(DevicePath::new(v))
            }
        }

        deserializer.deserialize_str(V)
    }
}

impl AsRef<DevicePath> for DevicePath {
    fn as_ref(&self) -> &DevicePath {
        self
    }
}

impl Into<DevicePath> for &DevicePath {
    fn into(self) -> DevicePath {
        DevicePath::new(self.raw_path.clone())
    }
}

impl DevicePath {
    /// Create a DevicePath from an unsquashed raw path
    pub fn new<T: Into<String>>(value: T) -> Self {
        let raw_path = value.into();
        let squashed_path =
            replace_char(&raw_path, OS_PATH_SEP_CHAR, REPLACED_DEVICE_PATH_SEP_CHAR);
        Self {
            raw_path,
            squashed_path,
        }
    }

    /// Create a device path from a squashed path
    pub fn from_squashed<T: Into<String>>(value: T) -> Self {
        let squashed_path = value.into();
        let raw_path = unreplace_char(
            &squashed_path,
            OS_PATH_SEP_CHAR,
            REPLACED_DEVICE_PATH_SEP_CHAR,
        );
        Self {
            raw_path,
            squashed_path,
        }
    }

    /// Takes a [Path] and parse the device path out of it, assuming the last
    /// part of the [Path] is a squashed path.
    pub fn from_path<P: AsRef<Path> + ?Sized>(value: &P) -> crate::Result<Self> {
        let path = value.as_ref();
        let safe_name = path
            .file_name()
            .ok_or_else(|| crate::Error::BadPath(PathBuf::from(path)))?
            .to_str()
            .ok_or_else(|| crate::Error::BadPath(PathBuf::from(path)))?
            .to_string();
        Ok(Self::from_squashed(safe_name))
    }

    /// Returns the file extension without the preceding `.`.
    pub fn extension(&self) -> Option<&str> {
        let fname = self.device_file_name();
        let (_, ext) = fname.rsplit_once('.')?;
        Some(ext)
    }

    /// Retrieve just the file name of the path
    pub fn device_file_name(&self) -> &str {
        match self.raw_path.rsplit_once(DEVICE_PATH_SEP_CHAR) {
            Some((_, fname)) => fname,
            None => &self.raw_path,
        }
    }

    /// Returns the path with [DEVICE_PATH_SEP_CHAR] as the path separator
    pub fn as_device_str(&self) -> &str {
        &self.raw_path
    }

    pub fn get_device_string(&self) -> String {
        self.as_device_str().to_string()
    }

    /// Returns the path with [REPLACED_DEVICE_PATH_SEP_CHAR] as the separator
    pub fn as_squashed_str(&self) -> &str {
        &self.squashed_path
    }

    /// Same as [as_squashed_str], but without any file extension
    pub fn as_squashed_str_no_ext(&self) -> &str {
        match self.squashed_path.split_once('.') {
            None => &self.squashed_path,
            Some((s, _)) => s,
        }
    }

    pub fn get_squashed_string(&self) -> String {
        self.as_squashed_str().to_string()
    }

    pub fn into_squashed(self) -> String {
        self.squashed_path
    }

    /// Squash the given device path into a squashed path
    pub fn squash<S: AsRef<str> + ?Sized>(device_path: &S) -> String {
        replace_char(
            device_path.as_ref(),
            OS_PATH_SEP_CHAR,
            REPLACED_DEVICE_PATH_SEP_CHAR,
        )
    }
}

// This is one of the main reasons for this type
impl AsRef<Path> for DevicePath {
    fn as_ref(&self) -> &Path {
        Path::new(self.as_squashed_str())
    }
}

impl AsRef<str> for DevicePath {
    fn as_ref(&self) -> &str {
        self.as_device_str()
    }
}

impl Debug for DevicePath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "DevicePath(\"{}\")", self.raw_path)
    }
}

impl Display for DevicePath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw_path)
    }
}

impl Hash for DevicePath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw_path.hash(state)
    }
}

impl PartialEq for DevicePath {
    fn eq(&self, other: &Self) -> bool {
        self.raw_path == other.raw_path
    }
}

#[cfg(feature = "sql")]
impl<DB> FromSql<Text, DB> for DevicePath
where
    DB: Backend,
    String: FromSql<Text, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let device_path = String::from_sql(bytes)?;
        Ok(Self::new(device_path))
    }
}

#[cfg(feature = "sql")]
impl<DB> ToSql<Text, DB> for DevicePath
where
    DB: Backend,
    String: ToSql<Text, DB>,
{
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, DB>) -> diesel::serialize::Result {
        self.raw_path.to_sql(out)
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_device_path_as_squashed_str() {
        let device_path = DevicePath::new("/system/priv-app/Test.apk");
        assert_eq!(device_path.as_squashed_str(), "%system%priv-app%Test.apk");
    }

    #[test]
    fn test_device_path_as_device_str() {
        let device_path = DevicePath::new("/system/priv-app/Test.apk");
        assert_eq!(device_path.as_device_str(), "/system/priv-app/Test.apk");
    }

    #[test]
    fn test_device_path_file_name() {
        let device_path = DevicePath::new("/system/priv-app/Test.apk");
        assert_eq!(device_path.device_file_name(), "Test.apk");
    }

    #[test]
    fn test_device_path_extension() {
        let device_path = DevicePath::new("/system/priv-app/Test.apk");
        assert_eq!(device_path.extension(), Some("apk"));
        let device_path = DevicePath::new("/system/priv-app/Test");
        assert_eq!(device_path.extension(), None);
    }

    #[test]
    fn test_device_path_from_path() {
        let path = PathBuf::from("test").join("%system%priv-app%Test.apk");
        let device_path = DevicePath::from_path(&path).unwrap();
        assert_eq!(device_path.as_device_str(), "/system/priv-app/Test.apk");
        assert_eq!(device_path.as_squashed_str(), "%system%priv-app%Test.apk");
    }

    #[test]
    fn test_device_path_from_squashed() {
        let device_path = DevicePath::from_squashed("%system%priv-app%Test\\%escape.apk");
        assert_eq!(
            device_path.as_device_str(),
            "/system/priv-app/Test%escape.apk"
        );
        assert_eq!(
            device_path.as_squashed_str(),
            "%system%priv-app%Test\\%escape.apk"
        );
    }

    #[test]
    fn test_device_path_simple() {
        let device_path = DevicePath::new("/system/priv-app/Test.apk");
        let path = PathBuf::from("test").join(&device_path);
        assert_eq!(path.file_name().unwrap(), "%system%priv-app%Test.apk");
    }

    #[test]
    fn test_device_path_escaping() {
        let device_path = DevicePath::new("/system/priv-app/Test%contains%replacement.apk");
        assert_eq!(
            device_path.as_squashed_str(),
            "%system%priv-app%Test\\%contains\\%replacement.apk"
        );
        let path = PathBuf::from("test").join(&device_path);
        assert_eq!(
            path.file_name().unwrap(),
            "%system%priv-app%Test\\%contains\\%replacement.apk"
        );
    }

    #[test]
    fn test_device_path() {
        let device_path_str = "/path/to/thing.jar";
        let squashed = "%path%to%thing.jar";

        let device_path = DevicePath::new(device_path_str);

        assert_eq!(device_path.as_squashed_str(), squashed);
        assert_eq!(device_path.get_squashed_string(), String::from(squashed));
        assert_eq!(device_path.as_device_str(), device_path_str);
        assert_eq!(
            device_path.get_device_string(),
            String::from(device_path_str)
        );
        assert_eq!(device_path.extension().expect("getting extension"), "jar");
        assert_eq!(device_path.device_file_name(), "thing.jar");

        let pb = PathBuf::from("test");
        let joined = pb.join(&device_path);
        assert_eq!(joined.file_name().expect("getting filename"), squashed);

        let from_path = DevicePath::from_path(&joined).expect("from_path");
        assert_eq!(from_path.as_device_str(), device_path_str);
        assert_eq!(from_path.as_squashed_str(), squashed);
    }
}
