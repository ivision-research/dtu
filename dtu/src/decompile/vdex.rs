use std::fs;
use std::path::Path;

use tempfile;

use crate::devicefs::DeviceFSHelper;
use crate::utils::path_must_str;
use crate::{run_cmd, Context};

use super::{Decompile, DecompileResult, DexFile};

#[cfg_attr(test, derive(PartialEq, Debug))]
pub struct VDexFile<'a> {
    source: &'a str,
}

impl<'a> VDexFile<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { source }
    }
}

impl<'a> Decompile for VDexFile<'a> {
    fn decompile(
        &self,
        ctx: &dyn Context,
        dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool> {
        let vdex_extractor = ctx.get_bin("vdexExtractor")?;
        let compact_dex_converter = ctx.get_bin("compact_dex_converter")?;
        let td = tempfile::Builder::new().prefix("dtu_vdex_").tempdir()?;
        let cdex_td = tempfile::Builder::new().prefix("dtu_cdex_").tempdir()?;

        let temp_out = path_must_str(&td.path());

        let args = [
            "-i",
            self.source,
            "-o",
            temp_out,
            "-f",
            "--ignore-crc-error",
        ];
        let res = run_cmd(&vdex_extractor, &args)?;

        // vdexExtractor isn't consistent with the exit code. We have to check
        // stdout (not stderr) for some key strings to determine if it worked

        let failed = res.stdout_contains("[ERROR]")
            || res.stdout_contains("[FATAL]")
            || res.stdout_contains("0 Dex files");

        if failed {
            return Ok(false);
        }

        let entries_it = fs::read_dir(&td)?;

        let entries = entries_it
            .filter(|ent| ent.is_ok())
            .map(|ent| ent.unwrap())
            .collect::<Vec<fs::DirEntry>>();

        // Consider a vdex file a failure if we didn't find anything
        if entries.len() == 0 {
            return Ok(false);
        }

        let cdex_temp_out = path_must_str(&cdex_td.path());

        for entry in entries {
            let pb = entry.path();
            let path = path_must_str(&pb);
            let args = ["-w", cdex_temp_out, path];

            run_cmd(&compact_dex_converter, &args)?;
        }

        let entries_it = fs::read_dir(&cdex_td)?;

        let entries = entries_it
            .filter(|ent| ent.is_ok())
            .map(|ent| ent.unwrap())
            .collect::<Vec<fs::DirEntry>>();

        // Consider a vdex file a failure compact_dex_converter failed
        if entries.len() == 0 {
            return Ok(false);
        }

        for entry in entries {
            let pb = entry.path();
            let path = path_must_str(&pb);
            let df = DexFile::new(path);
            df.decompile(ctx, dfs, out)?;
        }

        Ok(true)
    }
}
