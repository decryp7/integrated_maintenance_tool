//! Generates the gRPC code for `proto/control.proto` (and the test-only `proto/testing.proto`). `protox`
//! compiles the schemas in pure Rust, so building needs no `protoc`.

fn main() {
    let protos = ["proto/control.proto", "proto/testing.proto"];
    for p in protos {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    let fds = protox::compile(protos, ["proto"]).unwrap_or_else(|e| panic!("cannot compile the control protocol: {e}"));
    tonic_prost_build::configure()
        .compile_fds(fds)
        .unwrap_or_else(|e| panic!("cannot generate gRPC code: {e}"));
}
