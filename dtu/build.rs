use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{borrow::Cow, env, io::Write};

use anyhow::Context;
use zstd::Encoder;

fn visit_dir<W: Write, F: Fn(&Path) -> bool>(
    tf: &mut tar::Builder<W>,
    recurse: bool,
    dir: &Path,
    base: Option<&str>,
    filter: &Option<F>,
) -> anyhow::Result<()> {
    let paths = fs::read_dir(dir)
        .with_context(|| format!("reading dir: {:?}", dir))?
        .filter_map(|it| {
            let it = it.ok()?;
            let path = it.path();

            let keep = path.is_dir() || filter.as_ref().map(|it| it(&path)).unwrap_or(true);

            if keep {
                Some(path)
            } else {
                None
            }
        });

    for p in paths {
        let mut name = Cow::Borrowed(
            p.file_name()
                .expect("file names")
                .to_str()
                .expect("valid strings"),
        );

        if recurse {
            if let Some(b) = base {
                name = Cow::Owned(format!("{}/{}", b, name));
            }
            if p.is_dir() {
                visit_dir(tf, recurse, &p, Some(&name), &filter)?;
                continue;
            }
        } else if p.is_dir() {
            continue;
        }
        let mut file =
            File::open(&p).with_context(|| format!("opening file to add to tar: {:?}", p))?;

        tf.append_file(name.as_ref(), &mut file)
            .with_context(|| format!("adding {:?} to tar", p))?;
    }

    Ok(())
}

fn tar_dir<F: Fn(&Path) -> bool>(
    out: &str,
    dir: &str,
    recurse: bool,
    filter: Option<F>,
) -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed={dir}");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let in_path = manifest_dir.join(dir);

    let out_path = Path::new(&env::var("OUT_DIR").unwrap()).join(out);

    let out_file = OpenOptions::new()
        .truncate(true)
        .write(true)
        .create(true)
        .open(&out_path)
        .with_context(|| format!("opening output file: {:?}", out_path))?;

    let mut enc = Encoder::new(out_file, 19)?;
    enc.multithread(
        std::thread::available_parallelism()
            .unwrap_or(NonZero::new(1).unwrap())
            .get() as u32,
    )?;
    let enc = enc.auto_finish();
    let mut tf = tar::Builder::new(enc);

    visit_dir(&mut tf, recurse, &in_path, None, &filter)?;
    tf.finish()?;

    Ok(())
}

fn package_res_values() {
    tar_dir(
        "res.tar.zstd",
        "src/app/files/app/setup/res",
        true,
        None::<fn(&Path) -> bool>,
    )
    .unwrap();
}

fn package_aidls() {
    tar_dir(
        "aidl.tar.zstd",
        "src/app/files/app/setup",
        false,
        Some(|p: &Path| {
            if let Some(ext) = p.extension() {
                ext == "aidl"
            } else {
                false
            }
        }),
    )
    .unwrap();
}

fn package_app_sources() {
    tar_dir(
        "kt.tar.zstd",
        "src/app/files/app/setup",
        false,
        Some(|p: &Path| {
            if let Some(ext) = p.extension() {
                ext == "kt"
            } else {
                false
            }
        }),
    )
    .unwrap();
}

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    embed_version(&out_dir);
    package_app_sources();
    package_aidls();
    package_res_values();
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=build.rs");
}

fn embed_version(out_dir: &str) {
    let version = env!("CARGO_PKG_VERSION");
    let mut vparts = version.splitn(3, '.');
    let out_path = PathBuf::from(out_dir).join("current_version.rs");

    let major = vparts.next().expect("version major");
    let minor = vparts.next().expect("version minor");
    let patch_and_extra = vparts.next().expect("version patch");

    let git_rev = match get_git_rev() {
        Ok(v) => Cow::Owned(v),
        Err(_) => Cow::Borrowed("unknown"),
    };

    let (patch, extra) = if let Some((p, e)) = patch_and_extra.split_once('-') {
        (p, Cow::Owned(format!("Some(\"{e}\")")))
    } else {
        (patch_and_extra, Cow::Borrowed("None"))
    };

    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&out_path)
        .expect("create current version file");
    write!(
        f,
        "pub const VERSION: Version = Version {{ major: {major}, minor: {minor}, patch: {patch}, extra: {extra} }};\n"
    )
    .expect("failed to write VERSION");

    write!(
        f,
        "pub const GIT_COMMIT: &'static str = \"{}\";\n",
        git_rev.trim()
    )
    .expect("failed to write GIT_COMMIT");
}

fn get_git_rev() -> io::Result<String> {
    if let Ok(rev) = env::var("DTU_GIT_REVISION") {
        Ok(rev)
    } else {
        Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .and_then(|out| {
                String::from_utf8(out.stdout).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "git rev-parse HEAD output was not UTF-8",
                    )
                })
            })
            .or_else(|_| git_rev_from_file())
    }
}
fn git_rev_from_file() -> io::Result<String> {
    let git_file = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join(".git/refs/heads/main");
    let mut file = File::open(git_file)?;
    let mut rev = String::new();
    file.read_to_string(&mut rev)?;
    Ok(rev)
}
