#[cfg(feature = "setup")]
use std::collections::HashMap;
#[cfg(feature = "setup")]
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::{borrow::Cow, path::Path};

#[cfg(feature = "setup")]
use std::io::{BufRead, BufReader};

use walkdir::WalkDir;

use crate::command::LineCallback;
#[cfg(feature = "setup")]
use crate::db::sql::device::{DatabaseSetupHelper, PackageCallback, ServiceMeta};

use crate::devicefs::{FindLimits, FindName, FindType};
#[cfg(feature = "setup")]
use crate::utils::open_file;
use crate::utils::path_must_str;
use crate::{
    config::ConfigMap,
    devicefs::DeviceFSHelper,
    utils::{maybe_link, DevicePath},
    Context,
};

/// Implementation of device resources accessor traits via a filesystem dump
pub struct FSDumpAccess {
    base: PathBuf,
    pull_is_link: bool,
}

impl FSDumpAccess {
    pub(crate) const CONFIG_KEY: &'static str = "dump";

    pub fn new(base: PathBuf, pull_is_link: bool) -> Self {
        Self { base, pull_is_link }
    }

    pub fn from_cfg_map(ctx: &dyn Context, cfg: &ConfigMap) -> crate::Result<Self> {
        let path = cfg.must_get_str("base")?;
        let mut base = PathBuf::from(path);
        if base.is_relative() {
            base = ctx.get_project_dir()?.join(base);
        }
        let pull_is_link = cfg.get_bool_or("pull-is-link", false);
        Ok(Self::new(base, pull_is_link))
    }
}

impl FSDumpAccess {
    #[cfg(not(unix))]
    fn get_path<'a>(&'a self, mut device_path: &str) -> Cow<'a, Path> {
        // We don't really support non Unix systems, but one day maybe so when it's easy go ahead
        // and do it.

        if device_path.starts_with(path_must_str(&self.base)) {
            return Cow::Borrowed(Path::new(device_path));
        }

        while device_path.starts_with(crate::utils::DEVICE_PATH_SEP_CHAR) && device_path.len() > 0 {
            device_path = &device_path[1..device_path.len()];
        }
        if device_path.len() == 0 {
            return Cow::Borrowed(self.base.as_path());
        }

        // Borrow for now, might not need to do anything
        let mut path = Cow::Borrowed(Path::new(device_path));

        if device_path.contains(crate::utils::DEVICE_PATH_SEP_CHAR) {
            // Convert to our file separator
            let mut pb = PathBuf::new();

            for part in device_path.split(crate::utils::DEVICE_PATH_SEP_CHAR) {
                pb.push(&part);
            }

            path = Cow::Owned(pb);
        }

        Cow::Owned(self.base.join(path))
    }

    #[cfg(unix)]
    fn get_path<'a>(&'a self, mut device_path: &'a str) -> Cow<'a, Path> {
        // We may be given paths that we returned via `find`, so make sure it isn't already
        // inside the base dir

        if device_path.starts_with(path_must_str(&self.base)) {
            return Cow::Borrowed(Path::new(device_path));
        }

        // Fix absolute paths to become relative
        while device_path.starts_with(crate::utils::DEVICE_PATH_SEP_CHAR) && device_path.len() > 0 {
            device_path = &device_path[1..device_path.len()];
        }
        if device_path.len() == 0 {
            return Cow::Borrowed(self.base.as_path());
        }
        Cow::Owned(self.base.join(device_path))
    }
}

impl DeviceFSHelper for FSDumpAccess {
    fn pull(&self, device: &DevicePath, local: &str) -> crate::Result<()> {
        let full_path = self.get_path(device.as_device_str());

        if self.pull_is_link {
            maybe_link(&full_path, local)?;
        } else {
            let res = fs::copy(&full_path, local);
            if let Err(e) = res {
                log::error!("failed to copy {} to {}: {}", device, local, e);
                return Err(e.into());
            }
        }
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
        let path = self.get_path(dir);
        let mut wd = WalkDir::new(&path).follow_links(true);

        if let Some(limits) = limits {
            if let Some(min) = limits.mindepth {
                wd = wd.min_depth(min);
            }

            if let Some(max) = limits.maxdepth {
                wd = wd.max_depth(max);
            }
        }

        let filter = |it: Result<walkdir::DirEntry, walkdir::Error>| -> Option<PathBuf> {
            let f = it.ok()?;
            if !matches!(ty, FindType::Any) {
                let ft = f.file_type();

                if ft.is_dir() && !matches!(ty, FindType::Dir) {
                    return None;
                } else if ft.is_file() && !matches!(ty, FindType::File) {
                    return None;
                }
            }

            let path = f.path();

            if let Some(name) = &name {
                if !name.matches_path_file(path) {
                    return None;
                }
            }

            Some(path.into())
        };

        for elem in wd.into_iter().filter_map(filter) {
            let s = path_must_str(&elem);
            if let Err(e) = on_found(s) {
                return Err(crate::Error::Generic(e.to_string()));
            }
        }
        Ok(())
    }
}

#[cfg(feature = "setup")]
impl DatabaseSetupHelper for FSDumpAccess {
    fn get_props(&self) -> crate::Result<HashMap<String, String>> {
        // Loop for all .prop files and try to read the properties out of them. This will
        // miss dynamically set properties, but it's better than nothing.
        let mut map = HashMap::new();
        let mut line = String::new();

        let mut on_found = |filename: &str| -> anyhow::Result<()> {
            let f = match open_file(Path::new(filename)) {
                Err(e) => {
                    log::error!("failed to open file looking for properties: {}", e);
                    return Ok(());
                }
                Ok(f) => f,
            };

            let mut br = BufReader::new(f);

            loop {
                line.clear();
                let len = br.read_line(&mut line)?;
                if len == 0 {
                    break;
                }

                let trimmed = line.trim();
                if trimmed.starts_with("#") {
                    continue;
                }
                if let Some((key, val)) = trimmed.split_once('=') {
                    map.insert(key.into(), val.into());
                }
            }
            Ok(())
        };

        self.find(
            "/",
            FindType::File,
            Some(FindLimits::new_max(5)),
            Some(FindName::Suffix(".prop")),
            &mut on_found,
        )?;

        Ok(map)
    }

    fn list_services(&self) -> crate::Result<Vec<ServiceMeta>> {
        // We try to pull these out of the SELinux *_service_contexts files. This will
        // definitely include services that _don't_ exist on the device, but it should
        // also contain some (most? all?) that do exist.
        //
        // We could potentially try to remove false positives by looking for the service
        // name in various .so or .jar files, but that's a lot of nontrivial work.

        let mut service_set = HashSet::new();
        let mut line = String::new();

        let mut on_found = |filename: &str| -> anyhow::Result<()> {
            log::trace!("Found SELinux service_contexts file: {}", filename);
            let f = match open_file(Path::new(filename)) {
                Err(e) => {
                    log::error!("failed to open file looking for services: {}", e);
                    return Ok(());
                }
                Ok(f) => f,
            };

            let mut br = BufReader::new(f);

            loop {
                line.clear();
                let len = br.read_line(&mut line)?;
                if len == 0 {
                    break;
                }

                let trimmed = line.trim();
                if trimmed.len() == 0 || trimmed.starts_with("#") {
                    continue;
                }

                let mut idx = 0;
                for c in line.chars() {
                    if c.is_whitespace() {
                        break;
                    }
                    idx += 1;
                }

                let service = &trimmed[0..idx];
                if service == "*" {
                    continue;
                }
                log::trace!("Found service {service} in {filename}");

                service_set.insert(String::from(service));
            }

            Ok(())
        };

        self.find(
            "/",
            FindType::File,
            None,
            Some(FindName::Suffix("_service_contexts")),
            &mut on_found,
        )?;

        let mut services = Vec::new();
        services.extend(service_set.into_iter().map(|it| ServiceMeta {
            service_name: it,
            iface: None,
        }));
        Ok(services)
    }

    fn list_packages(&self, on_pkg: &mut PackageCallback) -> crate::Result<()> {
        self.find_apks(on_pkg)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::{HashMap, HashSet};

    use rstest::*;

    use crate::{
        devicefs::DeviceFSHelper,
        testing::{tmp_context, TestContext, TreeEntry},
    };

    #[fixture]
    #[once]
    fn devicefs_context(tmp_context: TestContext) -> TestContext {
        const SYSTEM_BUILD_PROP_CONTENT: &'static str = r#"ro.product.system.brand=test
ro.build.id=A.B.C.1
ro.build.display.id=A2.2025.1.A release-keys
# comment
ro.build.version.sdk=33
ro.empty="#;

        const VENDOR_BUILD_PROP_CONTENT: &'static str = r#"ro.vendor.id=asf
dalvik.vm.heapsize=512m
# comment
# comment
# ro.build.comment=asdf
ro.soc.model=ASDF
# end of file"#;

        const SYSTEM_PLAT_SERVICE_CONTEXTS: &'static str = r#"#line 1 "system/sepolicy/private/service_contexts"
android.hardware.audio.core.IConfig/default                          u:object_r:hal_audio_service:s0
# The instance here is internal/0 following naming convention for ICameraProvider.
# It advertises internal camera devices.
android.hardware.camera.provider.ICameraProvider/internal/0          u:object_r:hal_camera_service:s0

accessibility                             u:object_r:accessibility_service:s0
account                                   u:object_r:account_service:s0
window                                    u:object_r:window_service:s0
*                                         u:object_r:default_android_service:s0"#;
        const PRODUCT_SERVICE_CONTEXTS: &'static str = r#"#line 1 "device/google/sunfish-sepolicy/private/service_contexts"
qchook                                               u:object_r:qchook_service:s0
cneservice                                           u:object_r:cne_service:s0"#;

        tmp_context
            .create_tree(&[
                ("framework", TreeEntry::Dir),
                ("system/framework", TreeEntry::Dir),
                ("foo/framework", TreeEntry::Dir),
                ("apex", TreeEntry::Dir),
                ("system/apex", TreeEntry::Dir),
                ("a.apk", TreeEntry::EmptyFile),
                ("system/priv-app/b.apk", TreeEntry::EmptyFile),
                ("vendor/app/c.apk", TreeEntry::EmptyFile),
                ("foo/bar/baz/priv-app/d.apk", TreeEntry::EmptyFile),
                (
                    "system/build.prop",
                    TreeEntry::TxtFile(SYSTEM_BUILD_PROP_CONTENT),
                ),
                (
                    "vendor/build.prop",
                    TreeEntry::TxtFile(VENDOR_BUILD_PROP_CONTENT),
                ),
                (
                    "system/etc/selinux/plat_service_contexts",
                    TreeEntry::TxtFile(SYSTEM_PLAT_SERVICE_CONTEXTS),
                ),
                (
                    "product/etc/selinux/product_service_contexts",
                    TreeEntry::TxtFile(PRODUCT_SERVICE_CONTEXTS),
                ),
            ])
            .expect("creating base tree");

        tmp_context
    }

    #[rstest]
    fn test_fsdump_find_apks(devicefs_context: &TestContext) {
        let base = devicefs_context.get_base_dir();
        let fsd = FSDumpAccess::new(base.into(), false);

        let mut found = HashSet::new();

        let mut on_apk = |apk: &str| {
            found.insert(String::from(apk));
            Ok(())
        };

        let vexpected = vec![
            "a.apk",
            "system/priv-app/b.apk",
            "vendor/app/c.apk",
            "foo/bar/baz/priv-app/d.apk",
        ];

        let mut expected = HashSet::new();
        expected.extend(
            vexpected
                .into_iter()
                .map(|it| devicefs_context.to_abs_string(it)),
        );

        fsd.find_apks(&mut on_apk).unwrap();
        assert_eq!(found, expected);
    }

    #[rstest]
    fn test_fsdump_find_framework_dirs(devicefs_context: &TestContext) {
        let base = devicefs_context.get_base_dir();
        let fsd = FSDumpAccess::new(base.into(), false);

        let mut found = HashSet::new();

        let mut on_dir = |dir: &str| {
            found.insert(String::from(dir));
            Ok(())
        };

        let vexpected = vec!["system/framework", "foo/framework", "system/apex"];

        let mut expected = HashSet::new();
        expected.extend(
            vexpected
                .into_iter()
                .map(|it| devicefs_context.to_abs_string(it)),
        );

        fsd.find_framework_dirs(&mut on_dir).unwrap();
        assert_eq!(found, expected);
    }

    #[rstest]
    fn test_fsdump_getprop(devicefs_context: &TestContext) {
        let base = devicefs_context.get_base_dir();
        let fsd = FSDumpAccess::new(base.into(), false);

        let vexpected: Vec<(String, String)> = vec![
            ("ro.product.system.brand".into(), "test".into()),
            ("ro.build.id".into(), "A.B.C.1".into()),
            (
                "ro.build.display.id".into(),
                "A2.2025.1.A release-keys".into(),
            ),
            ("ro.build.version.sdk".into(), "33".into()),
            ("ro.empty".into(), "".into()),
            ("ro.vendor.id".into(), "asf".into()),
            ("dalvik.vm.heapsize".into(), "512m".into()),
            ("ro.soc.model".into(), "ASDF".into()),
        ];

        let mut expected: HashMap<String, String> = HashMap::new();
        expected.extend(vexpected.into_iter());

        let props = fsd.get_props().unwrap();
        assert_eq!(props, expected);
    }

    #[rstest]
    fn test_fsdump_list_services(devicefs_context: &TestContext) {
        let base = devicefs_context.get_base_dir();
        let fsd = FSDumpAccess::new(base.into(), false);

        let vexpected: Vec<String> = vec![
            "android.hardware.audio.core.IConfig/default".into(),
            "android.hardware.camera.provider.ICameraProvider/internal/0".into(),
            "accessibility".into(),
            "account".into(),
            "window".into(),
            "qchook".into(),
            "cneservice".into(),
        ];

        let mut expected: HashSet<ServiceMeta> = HashSet::new();
        expected.extend(vexpected.into_iter().map(|it| ServiceMeta {
            service_name: it,
            iface: None,
        }));
        let mut got: HashSet<ServiceMeta> = HashSet::new();

        let services = fsd.list_services().unwrap();
        got.extend(services.into_iter());
        assert_eq!(got, expected);
    }
}
