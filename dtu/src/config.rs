use std::{borrow::Cow, path::PathBuf};

use serde::Deserialize;

use crate::Context;

#[derive(Deserialize, Clone)]
pub struct AdbConfig {
    serial: Option<String>,
    executable: Option<String>,
}

impl Default for AdbConfig {
    fn default() -> Self {
        Self {
            serial: None,
            executable: None,
        }
    }
}

impl AdbConfig {
    pub fn get_serial(&self, ctx: &dyn Context) -> crate::Result<Cow<'_, str>> {
        match self.serial.as_ref() {
            Some(s) => Ok(Cow::Borrowed(s)),
            None => ctx.get_env("ANDROID_SERIAL").map(Cow::Owned),
        }
    }

    pub fn get_executable(&self, ctx: &dyn Context) -> crate::Result<Cow<'_, str>> {
        match self.executable.as_ref() {
            Some(s) => Ok(Cow::Borrowed(s)),
            None => ctx.get_bin("adb").map(Cow::Owned),
        }
    }
}

const fn bool_true() -> bool {
    true
}

const fn bool_false() -> bool {
    false
}

#[derive(Deserialize, Clone)]
pub struct DumpConfig {
    pub base: PathBuf,
    #[serde(rename = "pull-is-link", default = "bool_false")]
    pub pull_is_link: bool,
}

#[derive(Deserialize, Clone)]
pub enum DeviceAccessConfig {
    #[serde(rename = "adb")]
    Adb(AdbConfig),
    #[serde(rename = "dump")]
    Dump(DumpConfig),
}

impl Default for DeviceAccessConfig {
    fn default() -> Self {
        Self::Adb(AdbConfig::default())
    }
}

#[derive(Deserialize, Clone)]
pub struct ProjectConfig {
    #[serde(rename = "can-adb", default = "bool_true")]
    pub can_adb: bool,

    #[serde(rename = "device-access", default = "DeviceAccessConfig::default")]
    pub device_access: DeviceAccessConfig,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            can_adb: true,
            device_access: DeviceAccessConfig::default(),
        }
    }
}

impl ProjectConfig {
    pub fn is_adb(&self) -> bool {
        matches!(self.device_access, DeviceAccessConfig::Adb(_))
    }

    pub fn get_adb_bin(&self, ctx: &dyn Context) -> crate::Result<Cow<'_, str>> {
        if !self.can_adb {
            return Err(crate::Error::AdbDisabled);
        }

        let DeviceAccessConfig::Adb(adb) = &self.device_access else {
            return ctx.get_bin("adb").map(Cow::Owned);
        };

        adb.get_executable(ctx)
    }
}

#[derive(Clone, Deserialize)]
pub struct LocalFileStoreConfig {
    pub base: PathBuf,
    #[serde(rename = "get-is-link", default = "bool_false")]
    pub get_is_link: bool,
}

#[derive(Clone, Deserialize)]
pub struct S3FileStoreConfig {
    pub bucket: String,
    profile: Option<String>,
    #[serde(rename = "aws-bin")]
    aws_bin: Option<String>,
    #[serde(default = "bool_true")]
    pub cache: bool,
    #[serde(rename = "cache-is-link", default = "bool_false")]
    pub cache_is_link: bool,
}

impl S3FileStoreConfig {
    pub fn get_profile(&self, ctx: &dyn Context) -> Cow<'_, str> {
        match &self.profile {
            Some(v) => Cow::Borrowed(v),
            None => ctx
                .get_env("DTU_S3_PROFILE")
                .map(Cow::Owned)
                .unwrap_or_else(|_| Cow::Borrowed("dtu")),
        }
    }

    pub fn get_aws_bin(&self, ctx: &dyn Context) -> crate::Result<Cow<'_, str>> {
        match &self.aws_bin {
            Some(v) => Ok(Cow::Borrowed(v)),
            None => ctx.get_bin("aws").map(Cow::Owned),
        }
    }
}

impl LocalFileStoreConfig {
    pub fn new(base: PathBuf, get_is_link: bool) -> Self {
        Self { base, get_is_link }
    }

    fn get_default(ctx: &dyn Context) -> crate::Result<Self> {
        let base = ctx.get_user_local_dir()?.join("filestore");
        let get_is_link = false;
        Ok(Self::new(base, get_is_link))
    }
}

#[derive(Clone, Deserialize)]
pub enum FileStoreConfig {
    #[serde(rename = "local")]
    Local(LocalFileStoreConfig),
    #[serde(rename = "s3")]
    S3(S3FileStoreConfig),
}

impl FileStoreConfig {
    fn get_default(ctx: &dyn Context) -> crate::Result<Self> {
        Ok(Self::Local(LocalFileStoreConfig::get_default(ctx)?))
    }
}

#[derive(Clone, Deserialize)]
pub struct GlobalConfig {
    pub filestore: FileStoreConfig,
}

impl GlobalConfig {
    pub fn get_default(ctx: &dyn Context) -> crate::Result<Self> {
        Ok(Self {
            filestore: FileStoreConfig::get_default(ctx)?,
        })
    }
}

#[cfg(test)]
mod test {

    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::testing::{mock_context, MockContext};

    use super::*;
    use rstest::*;

    #[rstest]
    fn test_project_config_adb(mut mock_context: MockContext) {
        let raw_config = r#"
[device-access.adb]
serial = "DEVICE_SERIAL"
executable = "/path/to/adb"
"#;

        let config: ProjectConfig = toml::from_str(raw_config).expect("parse config");
        assert!(config.can_adb, "should have adb enabled by default");

        let DeviceAccessConfig::Adb(adb) = config.device_access else {
            panic!("should be adb device access");
        };
        assert_eq!(adb.get_serial(&mock_context).unwrap(), "DEVICE_SERIAL");
        assert_eq!(adb.get_executable(&mock_context).unwrap(), "/path/to/adb");

        let raw_config = "";
        let config: ProjectConfig = toml::from_str(raw_config).expect("parse config");
        assert!(config.can_adb, "should have adb enabled by default");

        let DeviceAccessConfig::Adb(adb) = config.device_access else {
            panic!("should default to adb device access");
        };

        let first_ser = AtomicBool::new(true);
        let first_bin = AtomicBool::new(true);

        mock_context.expect_maybe_get_env().returning(move |e| {
            if e == "ANDROID_SERIAL" {
                if first_ser.load(Ordering::Relaxed) {
                    first_ser.store(false, Ordering::Relaxed);
                    None
                } else {
                    Some(String::from("suchserial"))
                }
            } else {
                None
            }
        });

        mock_context.expect_maybe_get_bin().returning(move |b| {
            if b == "adb" {
                if first_bin.load(Ordering::Relaxed) {
                    first_bin.store(false, Ordering::Relaxed);
                    None
                } else {
                    Some(String::from("/usr/bin/adb"))
                }
            } else {
                None
            }
        });

        assert!(adb.get_serial(&mock_context).is_err());
        assert!(adb.get_executable(&mock_context).is_err());

        assert_eq!(adb.get_serial(&mock_context).unwrap(), "suchserial");
        assert_eq!(adb.get_executable(&mock_context).unwrap(), "/usr/bin/adb");
    }

    #[rstest]
    fn test_project_config_dump() {
        let raw_config = r#"
can-adb = false
[device-access.dump]
base = "/path/to/fs/dump/root"
pull-is-link = false
"#;

        let config: ProjectConfig = toml::from_str(raw_config).expect("parse config");
        assert!(!config.can_adb, "adb should be disabled");

        let DeviceAccessConfig::Dump(dump) = config.device_access else {
            panic!("should have been dump device access");
        };
        assert_eq!(dump.base, PathBuf::from("/path/to/fs/dump/root"));
        assert!(!dump.pull_is_link);
    }

    #[rstest]
    fn test_global_config_s3(mock_context: MockContext) {
        let raw = r#"[filestore.s3]
bucket = "neato"
aws-bin = "aws"
"#;
        let config: GlobalConfig = toml::from_str(&raw).expect("parse config");
        let FileStoreConfig::S3(s3) = config.filestore else {
            panic!("should have been a file store config");
        };

        assert_eq!(s3.get_aws_bin(&mock_context).unwrap(), "aws");
        assert_eq!(s3.bucket, "neato");
    }
}
