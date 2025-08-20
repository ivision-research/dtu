use std::path::Path;

use crate::devicefs::DeviceFSHelper;
use crate::Context;

use super::{
    ApexFile, ApkFile, Decompile, DecompileError, DecompileResult, DexFile, JarFile, VDexFile,
};

/// Wrapper for all of the decompile file types defined in this crate
#[cfg_attr(test, derive(PartialEq, Debug))]
pub enum DecompileFile<'a> {
    Apk(ApkFile<'a>),
    Jar(JarFile<'a>),
    VDex(VDexFile<'a>),
    Dex(DexFile<'a>),
    Apex(ApexFile<'a>),
}

impl<'a> DecompileFile<'a> {
    pub fn new(source: &'a str) -> Option<DecompileFile<'a>> {
        let (_, ext) = source.rsplit_once('.')?;
        Some(match ext {
            "jar" => Self::Jar(JarFile::new(source)),
            "apk" => Self::Apk(ApkFile::new(source)),
            "vdex" => Self::VDex(VDexFile::new(source)),
            "dex" => Self::Dex(DexFile::new(source)),
            "apex" => Self::Apex(ApexFile::new(source)),
            _ => return None,
        })
    }
}

impl<'a> Decompile for DecompileFile<'a> {
    fn decompile(
        &self,
        ctx: &dyn Context,
        dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool> {
        match self {
            Self::Apk(apk) => apk.decompile(ctx, dfs, out),
            Self::Jar(jar) => jar.decompile(ctx, dfs, out),
            Self::VDex(vdex) => vdex.decompile(ctx, dfs, out),
            Self::Dex(dex) => dex.decompile(ctx, dfs, out),
            Self::Apex(apex) => apex.decompile(ctx, dfs, out),
        }
    }
}

/// Helper function to decompile a file type that is known by this crate
pub fn decompile_file(
    ctx: &dyn Context,
    dfs: &dyn DeviceFSHelper,
    source: &str,
    out: &Path,
) -> DecompileResult<bool> {
    DecompileFile::new(source)
        .ok_or_else(|| DecompileError::InvalidFile)?
        .decompile(ctx, dfs, out)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_new_decompile_file() {
        let vdex = DecompileFile::new("test.vdex");
        assert_eq!(vdex, Some(DecompileFile::VDex(VDexFile::new("test.vdex"))));

        let dex = DecompileFile::new("test.dex");
        assert_eq!(dex, Some(DecompileFile::Dex(DexFile::new("test.dex"))));

        let jar = DecompileFile::new("test.jar");
        assert_eq!(jar, Some(DecompileFile::Jar(JarFile::new("test.jar"))));

        let apk = DecompileFile::new("test.apk");
        assert_eq!(apk, Some(DecompileFile::Apk(ApkFile::new("test.apk"))));

        let apex = DecompileFile::new("test.apex");
        assert_eq!(apex, Some(DecompileFile::Apex(ApexFile::new("test.apex"))));

        assert!(DecompileFile::new("test.cpp").is_none());
    }
}
