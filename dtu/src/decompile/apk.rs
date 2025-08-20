use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::Path;
use std::process::Child;
use std::{fs, io};

use super::{Decompile, DecompileResult};
use log;

use crate::command::{run_cmd, spawn_cmd};
use crate::devicefs::DeviceFSHelper;
use crate::utils::{ensure_dir_exists, path_has_ext, path_must_name, path_must_str, OS_PATH_SEP};
use crate::Context;

#[cfg_attr(test, derive(PartialEq, Debug))]
pub struct ApkFile<'a> {
    source: &'a str,
    force: bool,
    frameworks_path: Option<&'a str>,
    max_parallel: usize,
}

impl<'a> ApkFile<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            force: true,
            frameworks_path: None,
            max_parallel: 2,
        }
    }

    pub fn set_max_parallel(mut self, max_parallel: usize) -> Self {
        if max_parallel >= 1 {
            self.max_parallel = max_parallel;
        }
        self
    }

    pub fn set_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    pub fn set_frameworks_path(mut self, path: &'a str) -> Self {
        self.frameworks_path = Some(path);
        self
    }
}

impl<'a> Decompile for ApkFile<'a> {
    fn decompile(
        &self,
        ctx: &dyn Context,
        _dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool> {
        let out_path = out.as_ref();
        let success = match self.decompile_with_apktool(ctx, out_path) {
            Err(e) => {
                log::debug!("apktool failed, falling back to jadx, error: {}", e);
                false
            }
            Ok(v) => v,
        };

        if success {
            return Ok(true);
        }

        // TODO Should probably cleanup apktool output..?

        self.decompile_with_jadx(ctx, out_path)
    }
}

impl<'a> ApkFile<'a> {
    fn decompile_with_apktool(&self, ctx: &dyn Context, out: &Path) -> DecompileResult<bool> {
        let apktool = ctx.get_bin("apktool")?;
        let api_level = ctx.get_target_api_level().to_string();
        let dest = out.to_string_lossy();
        let mut args = vec!["-api", &api_level, "d", self.source, "-o", &dest];
        if self.force {
            args.push("-f")
        }
        if let Some(path) = self.frameworks_path {
            args.extend(&["-p", path])
        }
        let res = run_cmd(&apktool, &args)?;
        Ok(res.ok())
    }

    fn decompile_with_jadx(&self, ctx: &dyn Context, out: &Path) -> DecompileResult<bool> {
        // TODO Not using a tempdir here because fs::rename(...) doesn't work
        //  across file systems and could cause errors.

        let jadx_out = out.join("jadx_output");
        ensure_dir_exists(&jadx_out)?;

        if !self.run_jadx(ctx, &jadx_out)? {
            log::trace!("jadx ran without error but didn't succeed");
            return Ok(false);
        }
        let resources_dir = jadx_out.join("resources");
        self.move_jadx_output(&resources_dir, out)?;
        self.decompile_jadx_dex_files(ctx, &resources_dir, out)
    }

    fn run_jadx(&self, ctx: &dyn Context, out_dir: &Path) -> DecompileResult<bool> {
        let jadx = ctx.get_bin("jadx")?;
        let out_dir_str = out_dir.as_os_str().to_str().expect("valid paths");
        // jadx doesn't like paths that start with `@` :(
        let source = if self.source.starts_with("@") {
            Cow::Owned(format!(".{}{}", OS_PATH_SEP, self.source))
        } else {
            Cow::Borrowed(self.source)
        };
        let args: [&str; 4] = ["-s", "-d", out_dir_str, source.as_ref()];
        let res = run_cmd(&jadx, &args)?;
        Ok(res.ok())
    }

    fn move_jadx_output(&self, jadx_resources_dir: &Path, to_dir: &Path) -> DecompileResult<()> {
        const ANDROID_MANIFEST: &'static str = "AndroidManifest.xml";
        let mut src = jadx_resources_dir.join(ANDROID_MANIFEST);
        let mut dst = to_dir.join(ANDROID_MANIFEST);

        if src.exists() {
            fs::rename(&src, &dst)?;
        } else if dst.exists() {
            fs::remove_file(&dst)?;
        }

        macro_rules! move_dir {
            ($dst:expr, $src:expr, $path:expr) => {{
                $dst.pop();
                $src.pop();

                $dst.push($path);
                $src.push($path);

                if $dst.exists() {
                    fs::remove_dir_all(&$dst)?;
                }

                if $src.exists() {
                    fs::rename(&$src, &$dst)
                } else {
                    Ok(())
                }
            }};
        }

        move_dir!(dst, src, "res")?;
        move_dir!(dst, src, "assets")?;
        move_dir!(dst, src, "lib")?;

        Ok(())
    }

    fn decompile_jadx_dex_files(
        &self,
        ctx: &dyn Context,
        jadx_resources_dir: &Path,
        to_dir: &Path,
    ) -> DecompileResult<bool> {
        let dex_files = fs::read_dir(&jadx_resources_dir)?
            .filter(|it| {
                it.as_ref().map_or(false, |e| {
                    let path = e.path();
                    path.is_file()
                        && path_must_name(&path).starts_with("classes")
                        && path_has_ext(&path, "dex")
                })
            })
            .map(|it| it.unwrap());

        let baksmali = ctx.get_bin("baksmali")?;
        let api_level = ctx.get_target_api_level().to_string();

        let mut failed = false;
        let mut children = VecDeque::with_capacity(self.max_parallel);
        let mut active = 0;

        // Looping through everything and just storing "failed" as a final
        // return value
        for df in dex_files {
            while active >= self.max_parallel {
                let (count, success) = self.wait_for_children(&mut children)?;
                active -= count;
                if !success {
                    failed = true;
                }
            }

            children.push_back(self.decompile_jadx_dex_file(
                &df.path(),
                &baksmali,
                &api_level,
                to_dir,
            )?);
            active += 1;
        }

        // Need to wait for all processes
        for c in children.iter_mut() {
            let status = c.wait()?;
            if !failed && !status.success() {
                failed = true;
            }
        }

        Ok(!failed)
    }

    fn wait_for_children(&self, children: &mut VecDeque<Child>) -> io::Result<(usize, bool)> {
        let mut success = match children.pop_front() {
            Some(mut it) => {
                let res = it.wait()?;
                res.success()
            }
            None => return Ok((0, true)),
        };

        if children.len() == 0 {
            return Ok((1, success));
        }

        let mut count = 1;

        loop {
            match children.pop_front() {
                Some(mut it) => match it.try_wait()? {
                    Some(status) => {
                        count += 1;
                        if success && !status.success() {
                            success = false;
                        }
                    }

                    None => {
                        children.push_front(it);
                        break;
                    }
                },
                None => break,
            }
        }

        Ok((count, success))
    }

    fn decompile_jadx_dex_file(
        &self,
        dex_file: &Path,
        baksmali: &str,
        api_level: &str,
        to_dir: &Path,
    ) -> DecompileResult<Child> {
        let name = path_must_str(&dex_file);
        let fname = path_must_name(&dex_file);

        let number = get_class_file_number(fname);

        let out_file = match number {
            Some(v) => Cow::Owned(format!("smali_classes{}", v)),
            None => Cow::Borrowed("smali"),
        };

        let out_dir = to_dir.join(out_file.as_ref());
        let out_arg = path_must_str(&out_dir);

        Ok(spawn_cmd(
            baksmali,
            &["d", "--api", api_level, "-o", out_arg, name],
        )?)
    }
}

fn get_class_file_number(fname: &str) -> Option<u32> {
    const CLASSES_PREFIX_LEN: usize = "classes".len();
    let (_, without_classes) = fname.split_at(CLASSES_PREFIX_LEN);
    let dot_idx = without_classes.find('.')?;
    let (without_dex, _) = without_classes.split_at(dot_idx);
    str::parse::<u32>(without_dex).ok()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_class_file_number() {
        assert!(
            get_class_file_number("classes.dex").is_none(),
            "should have no number"
        );
        assert_eq!(get_class_file_number("classes1.dex"), Some(1));
        assert_eq!(get_class_file_number("classes25.dex"), Some(25));
    }
}
