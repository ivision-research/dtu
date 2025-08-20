use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use log::{self, log_enabled, Level::Trace};
use protobuf::{CodedInputStream, Message};
use protobuf_json_mapping::merge_from_str;
use tempfile;
use zip::ZipArchive;

use super::{ApkFile, Decompile, DecompileError, DecompileResult, JarFile};
use crate::devicefs::{DeviceFSHelper, FindName, FindType};
use crate::utils::fs::ensure_dir_exists;
use crate::utils::path_must_str;
use crate::Context;

use super::apex_manifest::ApexManifest;
use crate::utils::DevicePath;

type ApkCallback = Box<dyn Fn(&DevicePath, &PathBuf)>;

pub struct ApexFile<'a> {
    source: &'a str,
    apk_output_dir: Option<&'a PathBuf>,
    on_apk_output: Option<ApkCallback>,
}
#[cfg(test)]
impl<'a> PartialEq for ApexFile<'a> {
    fn eq(&self, other: &ApexFile<'a>) -> bool {
        self.source == other.source && self.apk_output_dir == other.apk_output_dir
    }
}

#[cfg(test)]
impl<'a> std::fmt::Debug for ApexFile<'a> {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        let on_apk_output = match &self.on_apk_output {
            Some(v) => format!("Some({:p})", v),
            None => String::from("None"),
        };

        f.debug_struct("ApexFile")
            .field("source", &self.source)
            .field("apk_output_dir", &self.apk_output_dir)
            .field("on_apk_output", &on_apk_output)
            .finish()
    }
}

impl<'a> ApexFile<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            apk_output_dir: None,
            on_apk_output: None,
        }
    }

    pub fn set_apk_output_callback(&mut self, callback: Option<ApkCallback>) {
        self.on_apk_output = callback;
    }

    pub fn set_apk_output_dir(&mut self, path: Option<&'a PathBuf>) {
        self.apk_output_dir = path;
    }
}

impl<'a> Decompile for ApexFile<'a> {
    fn decompile(
        &self,
        ctx: &dyn Context,
        dfs: &dyn DeviceFSHelper,
        out: &Path,
    ) -> DecompileResult<bool> {
        let fw_dir = ctx.get_frameworks_dir()?;
        ensure_dir_exists(&fw_dir)?;
        let apk_dir = ctx.get_apks_dir()?;
        ensure_dir_exists(&apk_dir)?;

        let td = tempfile::Builder::new().prefix("dtu_jar_").tempdir()?;
        let temp_path = td.path();
        if log_enabled!(Trace) {
            log::trace!(
                "unzipping apex file {} to {}",
                self.source,
                td.path().to_string_lossy()
            )
        }
        self.unzip(temp_path)?;
        let manifest = parse_manifest_file(temp_path)?;

        let name = &manifest.name;

        let mut on_jar = |jar_path: &str| -> anyhow::Result<()> {
            if jar_path.is_empty() {
                return Ok(());
            }
            let jar = DevicePath::new(jar_path);
            log::trace!("doing apex jar file {}", jar_path);
            let host_fs_path = fw_dir.join(&jar);
            let host_fs_file = host_fs_path.to_string_lossy();
            dfs.pull(&jar, &host_fs_file)?;
            JarFile::new(&host_fs_file).decompile(ctx, dfs, out)?;
            Ok(())
        };

        // This will fail when the DFS is adb backed since it will try to access
        // directories that the normal shell user won't have permission to search
        let _ = dfs.find(
            &format!("/apex/{}", name),
            FindType::File,
            None,
            Some(FindName::Suffix(".jar")),
            &mut on_jar,
        );

        let mut apk_out = if let Some(d) = self.apk_output_dir {
            d.clone()
        } else {
            let mut out_pb = out.to_path_buf();
            if !out_pb.pop() {
                out_pb = ctx.get_apks_dir()?.join("decompiled");
            } else {
                out_pb.push("apks");
            }
            out_pb
        };
        ensure_dir_exists(&apk_out)?;

        let framework_path_pb = ctx.get_output_dir_child("apktool-frameworks").ok();
        let use_framework = framework_path_pb.as_ref().map_or(false, |it| it.exists());
        let framework_path = if use_framework {
            framework_path_pb.as_ref().map(|it| path_must_str(it))
        } else {
            None
        };

        let mut on_apk = |apk_path: &str| -> anyhow::Result<()> {
            if apk_path.is_empty() {
                return Ok(());
            }
            let apk = DevicePath::new(apk_path);
            log::trace!("doing apex apk file {}", apk_path);
            let host_fs_path = apk_dir.join(&apk);
            let host_fs_file = host_fs_path.to_string_lossy();
            dfs.pull(&apk, &host_fs_file)?;
            apk_out.push(&apk);
            let mut apk_file = ApkFile::new(&host_fs_file).set_force(true);
            if let Some(path) = &framework_path {
                apk_file = apk_file.set_frameworks_path(path);
            }
            if apk_file.decompile(ctx, dfs, &apk_out)? {
                if let Some(callback) = &self.on_apk_output {
                    callback(&apk, &apk_out);
                }
            }
            apk_out.pop();
            Ok(())
        };

        // Ignoring error for the same reason as above
        let _ = dfs.find(
            &format!("/apex/{}", name),
            FindType::File,
            None,
            Some(FindName::Suffix(".apk")),
            &mut on_apk,
        );

        Ok(true)
    }
}

impl<'a> ApexFile<'a> {
    fn unzip(&self, td: &Path) -> DecompileResult<()> {
        let opened = match File::open(self.source) {
            Ok(f) => f,
            Err(e) => {
                log::error!("failed to open {}: {}", self.source, e);
                return Err(e.into());
            }
        };
        let mut archive = ZipArchive::new(&opened)?;
        let mut pb = td.to_path_buf();
        let nfiles = archive.len();
        for idx in 0..nfiles {
            let mut file = archive.by_index(idx)?;
            let fname = file.name();
            if !(fname.ends_with(".pb") || fname.ends_with("json")) {
                continue;
            }
            pb.push(file.name());
            let mut out_file = File::create(&pb)?;
            io::copy(&mut file, &mut out_file)?;
            pb.pop();
        }
        Ok(())
    }
}

fn parse_manifest_file(base_dir: &Path) -> DecompileResult<ApexManifest> {
    let mut pb = base_dir.to_path_buf();
    pb.push("apex_manifest.pb");
    if pb.exists() {
        log::trace!("parsing protobuf manifest");
        parse_manifest_file_pb(pb.as_path())
    } else {
        pb.pop();
        pb.push("apex_manifest.json");
        if pb.exists() {
            log::trace!("parsing apex_manifest");
            parse_manifest_file_json(pb.as_path())
        } else {
            Err(DecompileError::InvalidFile)
        }
    }
}

fn parse_manifest_file_pb(pb: &Path) -> DecompileResult<ApexManifest> {
    let mut man = ApexManifest::new();
    let mut f = File::open(pb)?;
    let mut input = CodedInputStream::new(&mut f);
    man.merge_from(&mut input)
        .map_err(|_e| DecompileError::InvalidFile)?;
    Ok(man)
}

fn parse_manifest_file_json(js: &Path) -> DecompileResult<ApexManifest> {
    let mut man = ApexManifest::new();
    let mut into = Vec::new();
    File::open(js)?.read_to_end(&mut into)?;
    let as_string = match String::from_utf8(into) {
        Ok(s) => s,
        _ => return Err(DecompileError::InvalidFile),
    };
    merge_from_str(&mut man, &as_string).map_err(|_e| DecompileError::InvalidFile)?;
    Ok(man)
}
