use std::path::Path;

use super::DecompileResult;
use crate::devicefs::DeviceFSHelper;
use crate::Context;

/// Decompile is the trait for all Android files that can be decompiled. This
/// includes:
///     - VDex files
///     - Jar files
///     - Dex files
///     - Apex files
/// and potentially others as the platform changes.
pub trait Decompile {
    /// Decompile the given file to the passed output path.
    ///
    /// Returns `Ok(true)` if the file was actually decompiled. It is possible
    /// that there were no errors running all of the associated commands, but
    /// no decompilation actually happened.
    fn decompile(
        &self,
        ctx: &dyn Context,
        dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool>;
}
