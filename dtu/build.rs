use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
    embed_version(&out_dir);
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=build.rs");
}

fn embed_version(out_dir: &str) {
    let version = env!("CARGO_PKG_VERSION");
    let mut vparts = version.splitn(3, '.');
    let out_path = PathBuf::from(out_dir).join("current_version.rs");
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&out_path)
        .expect("create current version file");
    write!(
        f,
        "pub const VERSION: Version = Version {{ major: {}, minor: {}, patch: {} }};\n",
        vparts.next().expect("version major"),
        vparts.next().expect("version minor"),
        vparts.next().expect("version patch"),
    )
    .expect("failed to write VERSION");
}
