use protobuf_codegen;

use std::env;
use std::fs;

fn main() {
    let mut cwd = env::current_dir().unwrap();

    cwd.push("generated-code");

    if !cwd.exists() {
        fs::create_dir(&cwd).unwrap();
    }

    protobuf_codegen::Codegen::new()
        .includes(&["protos"])
        .input("protos/apex_build_info.proto")
        .input("protos/apex_manifest.proto")
        .out_dir(&cwd)
        .run_from_script();
}
