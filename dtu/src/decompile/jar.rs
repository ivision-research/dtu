use std::fs::{self, File};
use std::io;
use std::path::Path;

use log::{self, log_enabled};
use tempfile;
use zip::ZipArchive;

use crate::devicefs::DeviceFSHelper;
use crate::Context;

use super::{Decompile, DecompileResult, DexFile};

#[cfg_attr(test, derive(PartialEq, Debug))]
pub struct JarFile<'a> {
    source: &'a str,
}

impl<'a> JarFile<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { source }
    }
}

impl<'a> Decompile for JarFile<'a> {
    fn decompile(
        &self,
        ctx: &dyn Context,
        _dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool> {
        let baksmali = ctx.get_bin("baksmali")?;
        let td = tempfile::Builder::new().prefix("dtu_jar_").tempdir()?;
        if log_enabled!(log::Level::Trace) {
            log::trace!(
                "unzipping jar file {} to {}",
                self.source,
                td.path().to_string_lossy()
            );
        }
        self.unzip(td.path())?;
        let entries_it = fs::read_dir(&td)?;

        let entries = entries_it
            .filter(|ent| ent.is_ok())
            .map(|ent| ent.unwrap())
            .collect::<Vec<fs::DirEntry>>();

        // Consider a jar file a failure if there were no dex files
        if entries.len() == 0 {
            return Ok(false);
        }

        let api_level = ctx.get_target_api_level().to_string();
        let mut success = true;
        for entry in entries {
            let path = entry.path();
            let path_str = path.to_string_lossy();
            let df = DexFile::new(&path_str);
            success |= df.do_decompile(&baksmali, &api_level, &out)?;
        }
        Ok(success)
    }
}

impl<'a> JarFile<'a> {
    fn unzip(&self, td: &Path) -> DecompileResult<()> {
        let opened = File::open(self.source)?;
        let mut archive = ZipArchive::new(&opened)?;
        let mut pb = td.to_path_buf();
        let nfiles = archive.len();
        for idx in 0..nfiles {
            let mut file = archive.by_index(idx)?;
            if !file.name().ends_with(".dex") {
                log::trace!("ignoring file: {}", file.name());
                continue;
            }
            pb.push(file.name());
            if log_enabled!(log::Level::Trace) {
                log::trace!("writing {} to {}", file.name(), pb.to_string_lossy());
            }
            let mut out_file = File::create(&pb)?;
            io::copy(&mut file, &mut out_file)?;
            pb.pop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::devicefs::AdbDeviceFS;
    use crate::errors::Error;
    use crate::testing::*;

    use crate::decompile::DecompileError;
    use rstest::*;

    #[rstest]
    fn test_decompile_jarfile_fails_no_bin(mut mock_context: MockContext, mock_adb: MockAdb) {
        mock_context.expect_maybe_get_bin().returning(|_| None);
        let jf = JarFile::new("test.jar");
        let dfs = AdbDeviceFS::new(mock_adb);
        let res = jf.decompile(&mock_context, &dfs, Path::new("unused"));
        match res {
            Err(DecompileError::PrereqError(Error::MissingBin(_))) => {}
            _ => panic!("should have errored but got {:?}", res),
        }
    }
}
