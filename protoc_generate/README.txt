Generate the protobuf code for Apex files. This is a separate executable so we
can just copy the output over to `dtu/src/decompile` and save it there instead
of requiring protoc for building the crate.
