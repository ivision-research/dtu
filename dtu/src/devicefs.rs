use crate::{
    adb::{Adb, ExecAdb, ADB_CONFIG_KEY},
    command::{quote, LineCallback},
    fsdump::FSDumpAccess,
    utils::DevicePath,
    Context,
};
use std::{borrow::Cow, ops::Deref, path::Path};

/// Trait for getting files off the device
///
/// This trait is implemented for any [Adb] implementation but is also implemented
/// for a local filesystem tree so device filesystem dumps can be used
pub trait DeviceFSHelper: Send + Sync {
    fn pull(&self, device: &DevicePath, local: &str) -> crate::Result<()>;

    fn find(
        &self,
        dir: &str,
        ty: FindType,
        limits: Option<FindLimits>,
        name: Option<FindName>,
        on_found: &mut LineCallback,
    ) -> crate::Result<()>;

    fn find_files_in_dir(&self, dir: &str, on_file: &mut LineCallback) -> crate::Result<()> {
        self.find(dir, FindType::File, None, None, on_file)
    }

    fn find_framework_dirs(&self, on_dir: &mut LineCallback) -> crate::Result<()> {
        self.find(
            "/",
            FindType::Dir,
            Some(FindLimits::new_min_max(2, 4)),
            Some(FindName::Exact("framework")),
            on_dir,
        )?;
        self.find(
            "/",
            FindType::Dir,
            Some(FindLimits::new_min_max(2, 4)),
            Some(FindName::Exact("apex")),
            on_dir,
        )?;
        Ok(())
    }

    fn find_apks(&self, on_apk: &mut LineCallback) -> crate::Result<()> {
        self.find(
            "/",
            FindType::File,
            None,
            Some(FindName::Suffix(".apk")),
            on_apk,
        )
    }
}

#[derive(Clone, Copy)]
pub enum FindType {
    Any,
    File,
    Dir,
}

pub struct FindLimits {
    pub mindepth: Option<usize>,
    pub maxdepth: Option<usize>,
}

impl FindLimits {
    pub fn new(mindepth: Option<usize>, maxdepth: Option<usize>) -> Self {
        Self { mindepth, maxdepth }
    }

    pub fn new_min_max(min: usize, max: usize) -> Self {
        Self::new(Some(min), Some(max))
    }

    pub fn new_min(min: usize) -> Self {
        Self::new(Some(min), None)
    }
    pub fn new_max(max: usize) -> Self {
        Self::new(None, Some(max))
    }
}

pub enum FindName<'a> {
    /// Search by suffix: -name '*.jar'
    Suffix(&'a str),
    /// Search by prefix: -name 'foo*'
    Prefix(&'a str),
    /// Search for exact: -name 'bar'
    Exact(&'a str),
    /// Search for exact case insensitive: -iname 'foo'
    CaseInsensitive(&'a str),
}

impl<'a> FindName<'a> {
    pub fn matches_path_file(&self, path: &Path) -> bool {
        let name = match path.file_name() {
            None => return false,
            Some(v) => v,
        };

        let as_str = name.to_string_lossy();

        match self {
            Self::Suffix(suff) => as_str.ends_with(suff),
            Self::Prefix(pre) => as_str.starts_with(pre),
            Self::Exact(s) => as_str == *s,
            Self::CaseInsensitive(s) => {
                if as_str.eq_ignore_ascii_case(s) {
                    true
                } else {
                    as_str.to_lowercase() == s.to_lowercase()
                }
            }
        }
    }

    pub fn to_globbed(&self) -> Cow<'a, str> {
        match self {
            Self::Suffix(suff) => Cow::Owned(format!("*{}", suff)),
            Self::Prefix(pre) => Cow::Owned(format!("{}*", pre)),
            Self::Exact(s) | Self::CaseInsensitive(s) => Cow::Borrowed(s),
        }
    }
}

impl<T> DeviceFSHelper for Box<T>
where
    T: DeviceFSHelper + ?Sized,
{
    fn pull(&self, device: &DevicePath, local: &str) -> crate::Result<()> {
        self.as_ref().pull(device, local)
    }

    fn find(
        &self,
        dir: &str,
        ty: FindType,
        limits: Option<FindLimits>,
        name: Option<FindName>,
        on_found: &mut LineCallback,
    ) -> crate::Result<()> {
        self.as_ref().find(dir, ty, limits, name, on_found)
    }

    fn find_apks(&self, on_apk: &mut LineCallback) -> crate::Result<()> {
        self.as_ref().find_apks(on_apk)
    }

    fn find_files_in_dir(&self, dir: &str, on_file: &mut LineCallback) -> crate::Result<()> {
        self.as_ref().find_files_in_dir(dir, on_file)
    }

    fn find_framework_dirs(&self, on_dir: &mut LineCallback) -> crate::Result<()> {
        self.as_ref().find_framework_dirs(on_dir)
    }
}

/// Wrapper for types that implement Adb
pub struct AdbDeviceFS<T: Adb>(T);

impl<T> Deref for AdbDeviceFS<T>
where
    T: Adb,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> AdbDeviceFS<T>
where
    T: Adb,
{
    pub fn new(adb: T) -> Self {
        Self(adb)
    }
}

impl<T> AdbDeviceFS<T>
where
    T: Adb,
{
    pub fn as_adb(&self) -> &dyn Adb {
        &self.0
    }
}

impl<T> DeviceFSHelper for AdbDeviceFS<T>
where
    T: Adb,
{
    fn pull(&self, device: &DevicePath, local: &str) -> crate::Result<()> {
        Adb::pull(&self.0, device.as_device_str(), local)?;
        Ok(())
    }

    fn find(
        &self,
        dir: &str,
        ty: FindType,
        limits: Option<FindLimits>,
        name: Option<FindName>,
        on_found: &mut LineCallback,
    ) -> crate::Result<()> {
        let mut args = Vec::new();

        match ty {
            FindType::Dir => args.push(Cow::Borrowed("-type d")),
            FindType::File => args.push(Cow::Borrowed("-type f")),
            FindType::Any => {}
        }

        if let Some(v) = limits {
            if let Some(min) = v.mindepth {
                args.push(Cow::Owned(format!("-mindepth {}", min)));
            }

            if let Some(max) = v.maxdepth {
                args.push(Cow::Owned(format!("-maxdepth {}", max)));
            }
        }

        if let Some(name) = name {
            let val = quote(&name.to_globbed());

            let flag = match name {
                FindName::CaseInsensitive(_) => "-iname",
                _ => "-name",
            };

            args.push(Cow::Owned(format!("{} {}", flag, val)));
        }

        let full = format!(
            "find {} {} -print0 2> /dev/null",
            quote(dir),
            args.join(" ")
        );

        self.streamed_find_no_stderr(&full, on_found)?;

        Ok(())
    }

    fn find_apks(&self, on_apk: &mut LineCallback) -> crate::Result<()> {
        let mut on_serr = |line: &str| -> anyhow::Result<()> {
            log::warn!("`pm list packages -f` stderr: {}", line);
            Ok(())
        };

        let mut on_pm_list_line = |line: &str| -> anyhow::Result<()> {
            let start = match line.find(':') {
                Some(v) => v + 1,
                None => {
                    log::error!("invalid output for list packages (no `:`): {}", line);
                    return Ok(());
                }
            };
            let end = match line.rfind('=') {
                Some(v) => v,
                None => {
                    log::error!("invalid output for list packages (no `=`): {}", line);
                    return Ok(());
                }
            };

            let apk_path = &line[start..end];
            if apk_path.len() == 0 {
                log::warn!("apk missing path for line: {}", line);
                return Ok(());
            }

            on_apk(apk_path)
        };

        // We search for APKs with both of these methods. While they're likely to contain some
        // overlap, we've seen differences in the two. While the `find / ...` takes a bit of time,
        // so does the entire pulling process so it's worth it for better coverage.

        self.shell_split_streamed(
            "pm list packages -f",
            b'\n',
            &mut on_pm_list_line,
            &mut on_serr,
        )?;

        self.streamed_find_no_stderr("find / -type f -name '*.apk' -print0 2>/dev/null", on_apk)?;

        Ok(())
    }
}

/// Get the DeviceFSHelper implementation for the current project
///
/// If no config is found or device-access is unspecified, this returns an AdbDeviceFS
///
/// Note that a `device-access` key is required if `can-adb` is false in the configuration.
pub fn get_project_devicefs_helper(ctx: &dyn Context) -> crate::Result<Box<dyn DeviceFSHelper>> {
    let config = match ctx.get_project_config()? {
        None => return Ok(Box::new(AdbDeviceFS::new(ExecAdb::new(ctx)?))),
        Some(v) => v,
    };
    let base = config.get_map();

    let cfg = match base.maybe_get_map_typecheck("device-access")? {
        None => {
            if base.get_bool_or("can-adb", true) {
                return Ok(Box::new(AdbDeviceFS::new(ExecAdb::new(ctx)?)));
            }
            return Err(config.invalid_error(
                "ADB disabled by configuration file and no device-access key".into(),
            ));
        }
        Some(v) => v,
    };

    if cfg.has(FSDumpAccess::CONFIG_KEY) && cfg.has(ADB_CONFIG_KEY) {
        return Err(config.invalid_error(format!(
            "`device-access` can't have both `{ADB_CONFIG_KEY}` and `{}` keys",
            FSDumpAccess::CONFIG_KEY
        )));
    }

    if cfg.has(FSDumpAccess::CONFIG_KEY) {
        log::trace!("Using dump config");
        let dump_cfg = cfg.must_get_map(FSDumpAccess::CONFIG_KEY)?;
        Ok(Box::new(FSDumpAccess::from_cfg_map(ctx, &dump_cfg)?))
    } else if cfg.has(ADB_CONFIG_KEY) {
        log::trace!("Using adb config");
        let adb_cfg = cfg.must_get_map(ADB_CONFIG_KEY)?;
        Ok(Box::new(AdbDeviceFS::new(ExecAdb::from_config(
            ctx, &adb_cfg,
        )?)))
    } else {
        Err(config.invalid_error(format!(
            "`device-access` needs either `{ADB_CONFIG_KEY}` or `{}` keys",
            FSDumpAccess::CONFIG_KEY
        )))
    }
}
