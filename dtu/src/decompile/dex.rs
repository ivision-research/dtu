use std::path::Path;

use super::{Decompile, DecompileResult};
use crate::{devicefs::DeviceFSHelper, run_cmd, Context};

#[cfg_attr(test, derive(PartialEq, Debug))]
pub struct DexFile<'a> {
    source: &'a str,
}

impl<'a> DexFile<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { source }
    }
    pub(crate) fn do_decompile<P: AsRef<Path> + ?Sized>(
        &self,
        baksmali: &str,
        api_level: &str,
        out: &P,
    ) -> DecompileResult<bool> {
        let out_str = out.as_ref().to_string_lossy();
        let args = ["d", "--api", api_level, self.source, "-o", &out_str];
        let res = run_cmd(&baksmali, &args)?;
        let ok = res.ok();
        if !ok {
            log::error!("baksmali failed: {}", res.stderr_utf8_lossy());
        }
        Ok(ok)
    }
}

impl<'a> Decompile for DexFile<'a> {
    #[inline]
    fn decompile(
        &self,
        ctx: &dyn Context,
        _dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool> {
        let baksmali = ctx.get_bin("baksmali")?;
        let api_level = ctx.get_target_api_level().to_string();
        self.do_decompile(&baksmali, &api_level, out)
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
    fn test_decompile_dexfile_fails_no_bin(mut mock_context: MockContext, mock_adb: MockAdb) {
        mock_context.expect_maybe_get_bin().returning(|_| None);
        let df = DexFile::new("test.dex");
        let dfs = AdbDeviceFS::new(mock_adb);
        let res = df.decompile(&mock_context, &dfs, Path::new("unused"));
        match res {
            Err(DecompileError::PrereqError(Error::MissingBin(_))) => {}
            _ => panic!("should have errored but got {:?}", res),
        }
    }
}
