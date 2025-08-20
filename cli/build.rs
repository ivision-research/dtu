use std::borrow::Cow;
use std::{
    env,
    fs::File,
    io::{self, Read, Write},
    path::Path,
    process::Command,
};

fn main() {
    match write_version_file() {
        Ok(_) => {}
        Err(e) => panic!("Failed to create a version file: {:?}", e),
    }
}

fn write_version_file() -> io::Result<()> {
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let simple_version_fname =
        Path::new(&env::var("OUT_DIR").unwrap()).join("simple_version_string");
    let mut simple_version_file = File::create(&simple_version_fname)?;
    write!(&mut simple_version_file, "\"{}\"", version)?;

    let target = env::var("TARGET").unwrap();
    let version_fname = Path::new(&env::var("OUT_DIR").unwrap()).join("version_string");
    let mut version_file = File::create(&version_fname)?;
    let git_rev = match get_git_rev() {
        Ok(v) => Cow::Owned(v),
        Err(_) => Cow::Borrowed("unknown"),
    };
    write!(
        &mut version_file,
        "r#\"{} ({})\nrev {}\"#",
        version,
        target,
        git_rev.trim(),
    )?;
    Ok(())
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
