use crate::utils::{ClassName, DevicePath};
use crate::Context;
use std::borrow::Cow;
use std::fs::{self, create_dir_all, read_dir, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use super::{replace_char, unreplace_char};

#[cfg(not(windows))]
pub const OS_PATH_SEP: &'static str = "/";
#[cfg(not(windows))]
pub const OS_PATH_SEP_CHAR: char = '/';

#[cfg(windows)]
pub const OS_PATH_SEP: &'static str = "\\";
#[cfg(windows)]
pub const OS_PATH_SEP_CHAR: char = '\\';

pub const DEVICE_PATH_SEP: &'static str = "/";
pub const DEVICE_PATH_SEP_CHAR: char = '/';

// Note that the REPLACED_DEVICE_PATH_SEP and the SQUASH_PATH_SEP have to be different

pub const REPLACED_DEVICE_PATH_SEP: &'static str = "%";
pub const REPLACED_DEVICE_PATH_SEP_CHAR: char = '%';

pub const SQUASH_PATH_SEP: &'static str = "#";
pub const SQUASH_PATH_SEP_CHAR: char = '#';

pub fn maybe_link<T: AsRef<Path> + ?Sized, U: AsRef<Path> + ?Sized>(
    from: &T,
    to: &U,
) -> io::Result<()> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(from, to)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(from, to)?;
    #[cfg(not(any(unix, windows)))]
    fs::copy(from, to)?;

    Ok(())
}

pub fn ensure_dir_exists(p: &Path) -> io::Result<()> {
    if p.exists() {
        return Ok(());
    }

    create_dir_all(p)
}

/// Unsquashes a path squashed with [squash_path].
///
/// Example:
///
/// DTU_PROJECT_HOME  is `/tmp/test`
/// path is `/very/cool/squashed#path#wow`
///
/// If `proj_relative` is true unsquash will return `/tmp/test/squashed/path/wow`.
/// If `proj_relative` is false, unsquash will return `squashed/path/wow`.
pub fn unsquash_path(ctx: &dyn Context, path: &Path, proj_relative: bool) -> Option<PathBuf> {
    let fname = path.file_name()?;
    let name = fname.to_str()?;
    let unsquashed = unreplace_char(name, OS_PATH_SEP_CHAR, SQUASH_PATH_SEP_CHAR);
    let pbuf = PathBuf::from(unsquashed);
    if !proj_relative || pbuf.is_absolute() {
        return Some(pbuf);
    }
    let home = ctx.get_project_dir().ok()?;
    Some(home.join(pbuf))
}

/// Squash a path by replacing [OS_PATH_SEP] with [SQUASH_PATH_SEP_CHAR].
///
/// Example:
///
/// DTU_PROJECT_HOME  is `/tmp/test`
/// path is `/tmp/test/wow/such/squashed`
///
/// If `proj_relative` is true squash will return `wow#such#squashed`.
/// If `proj_relative` is false, squash will return `#tmp#test#wow#such#squashed`.
pub fn squash_path(ctx: &dyn Context, path: &Path, proj_relative: bool) -> Option<String> {
    let squashed = if proj_relative {
        let home = ctx.get_project_dir().ok()?;
        let home_str = home.to_str()?;
        let pathstr = path.to_str()?;
        if pathstr.starts_with(home_str) {
            let (_, rel) = pathstr.split_at(home_str.len());
            Cow::Owned(rel.trim_start_matches(OS_PATH_SEP).into())
        } else {
            Cow::Borrowed(path.to_str()?)
        }
    } else {
        Cow::Borrowed(path.to_str()?)
    };
    Some(replace_char(
        &squashed,
        OS_PATH_SEP_CHAR,
        SQUASH_PATH_SEP_CHAR,
    ))
}

/// Searches the entire smali dir (as defined by the [Context]) for files
/// that implement the given class.
pub fn find_files_for_class(ctx: &dyn Context, class: &ClassName) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let smali_dir = match ctx.get_smali_dir().ok() {
        None => return files,
        Some(d) => d,
    };
    let framework = smali_dir.join("framework");
    let as_path =
        PathBuf::from(class.get_java_name().replace('.', OS_PATH_SEP)).with_extension("smali");
    let path = framework.join(&as_path);
    if path.exists() {
        log::trace!("class {} defined at path {:?}", class, path);
        files.push(path);
    }

    let apks = smali_dir.join("apks");

    let dirs = match read_dir(&apks) {
        Ok(d) => d
            .filter(|r| r.as_ref().map_or(false, |e| e.path().is_dir()))
            .map(|r| r.unwrap().path()),
        Err(_) => return files,
    };

    for d in dirs {
        let path = if d.is_absolute() {
            d.join(&as_path)
        } else {
            apks.join(d).join(&as_path)
        };
        if path.exists() {
            log::trace!("class {} defined at path {:?}", class, path);
            files.push(path);
        }
    }
    files
}

pub fn find_file_for_class(ctx: &dyn Context, class: &ClassName) -> Option<PathBuf> {
    find_files_for_class(ctx, class).into_iter().next()
}

pub fn try_proj_home_relative(ctx: &dyn Context, path: &Path) -> PathBuf {
    if path.is_relative() {
        return PathBuf::from(path);
    }
    let home = match ctx.get_project_dir() {
        Ok(v) => v,
        Err(_) => return PathBuf::from(path),
    };

    let home_str = home.to_str().expect("valid paths");
    let path = path.to_str().expect("valid paths");
    if path.starts_with(home_str) {
        let (_, rel) = path.split_at(home_str.len());
        PathBuf::from(rel.trim_start_matches(OS_PATH_SEP))
    } else {
        PathBuf::from(path)
    }
}

pub fn find_smali_file_for_class(
    ctx: &dyn Context,
    class_name: &ClassName,
    apk: Option<&DevicePath>,
) -> Option<PathBuf> {
    let mut base = ctx.get_smali_dir().ok()?;
    match apk {
        Some(apk) => {
            base.push("apks");
            base.push(apk);
        }
        None => base.push("framework"),
    }
    let as_java = class_name.get_java_name();
    let to_path = as_java.replace('.', OS_PATH_SEP);
    base.push(Path::new(&to_path));

    Some(base.with_extension("smali"))
}

/// Check to see if the given pathlike type has the given extension
pub fn path_has_ext<P: AsRef<Path> + ?Sized>(p: &P, ext: &str) -> bool {
    let path = p.as_ref();
    path.extension().map_or(false, |it| it == ext)
}

/// Calls `to_str` on the path and returns the string, panicking if that fails
pub fn path_must_str(path: &Path) -> &str {
    path.to_str().expect("valid paths")
}

/// Returns the filename of the path and panics if that fails
pub fn path_must_name(path: &Path) -> &str {
    path.file_name()
        .expect("valid paths")
        .to_str()
        .expect("valid paths")
}

pub fn open_file(path: &Path) -> crate::Result<File> {
    match File::open(path) {
        Ok(v) => Ok(v),
        Err(e) => match e.kind() {
            ErrorKind::NotFound => Err(crate::Error::MissingFile(path_must_str(path).into())),
            _ => Err(e.into()),
        },
    }
}

pub fn read_file(path: &Path) -> crate::Result<String> {
    match fs::read_to_string(path) {
        Ok(v) => Ok(v),
        Err(e) => match e.kind() {
            ErrorKind::NotFound => Err(crate::Error::MissingFile(path_must_str(path).into())),
            _ => Err(e.into()),
        },
    }
}

#[cfg(test)]
mod test {
    use crate::testing::{mock_context, MockContext};
    use crate::utils::{path_has_ext, squash_path, unsquash_path};
    use crate::Context;
    use std::path::PathBuf;

    use rstest::*;

    #[rstest]
    fn test_path_has_ext() {
        let path = PathBuf::from("path").join("to").join("test.apk");
        assert!(path_has_ext(&path, "apk"));
        let path = "/path/to/test.apk";
        assert!(path_has_ext(path, "apk"));
    }

    #[rstest]
    fn test_squash_path(mut mock_context: MockContext) {
        mock_context.expect_maybe_get_env().returning(|e| match e {
            "DTU_PROJECT_HOME" => Some(String::from("/tmp/test")),
            _ => None,
        });

        mock_context
            .expect_get_project_dir()
            .returning(|| Ok("/tmp/test".into()));

        let path = mock_context
            .get_project_dir()
            .expect("should get project dir")
            .join("wow")
            .join("such")
            .join("path");

        let squashed = squash_path(&mock_context, &path, true).expect("failed to squash path");

        assert_eq!(squashed.as_str(), "wow#such#path", "squash failed");

        let unsquashed = unsquash_path(&mock_context, &PathBuf::from(&squashed), true)
            .expect("failed to unsquash path");

        assert_eq!(unsquashed, path, "squash/unsquash mismatch");

        let squashed =
            squash_path(&mock_context, &path, false).expect("failed to squash no proj relative");

        assert_eq!(
            squashed.as_str(),
            "#tmp#test#wow#such#path",
            "squash failed"
        );
        let unsquashed = unsquash_path(&mock_context, &PathBuf::from(&squashed), true)
            .expect("failed to unsquash path");
        assert_eq!(unsquashed, path, "squash/unsquash mismatch");
    }
}
