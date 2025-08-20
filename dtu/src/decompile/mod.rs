mod apex_build_info;
mod apex_manifest;

mod decompile;
pub use decompile::Decompile;

mod apk;
pub use apk::ApkFile;
mod jar;
pub use jar::JarFile;
mod dex;
pub use dex::DexFile;
mod vdex;
pub use vdex::VDexFile;
mod apex;
pub use apex::ApexFile;

mod error;
pub use error::{DecompileError, DecompileResult};

mod decompile_file;

use crate::utils::DevicePath;
pub use decompile_file::{decompile_file, DecompileFile};

#[cfg_attr(debug_assertions, derive(Debug, Clone))]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FrameworkFileType {
    Jar = 0,
    VDex = 1,
    Dex = 2,
    Apex = 3,
    // TODO oat, odex, art?
    Apk = 255,
}

impl FrameworkFileType {
    /// Returns the [FrameworkFileType] for a given file if known
    pub fn from_device_path<T: AsRef<DevicePath> + ?Sized>(
        device_path: &T,
    ) -> Option<FrameworkFileType> {
        let device_path = device_path.as_ref();
        let ext = device_path.extension()?;
        Some(match ext {
            "jar" => Self::Jar,
            "vdex" => Self::VDex,
            "dex" => Self::Dex,
            "apex" => Self::Apex,
            "apk" => Self::Apk,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_from_device_path() {
        assert_eq!(
            FrameworkFileType::from_device_path(&DevicePath::new("test.jar")),
            Some(FrameworkFileType::Jar)
        );
        assert_eq!(
            FrameworkFileType::from_device_path(&DevicePath::new("test.dex")),
            Some(FrameworkFileType::Dex)
        );
        assert_eq!(
            FrameworkFileType::from_device_path(&DevicePath::new("test.apex")),
            Some(FrameworkFileType::Apex)
        );
        assert_eq!(
            FrameworkFileType::from_device_path(&DevicePath::new("test.apk")),
            Some(FrameworkFileType::Apk)
        );
        assert_eq!(
            FrameworkFileType::from_device_path(&DevicePath::new("/system/framework/test.vdex")),
            Some(FrameworkFileType::VDex)
        );
        assert_eq!(
            FrameworkFileType::from_device_path(&DevicePath::new("test.unknown")),
            None
        );
    }
}
