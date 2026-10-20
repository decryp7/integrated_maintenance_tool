//! Generates the gRPC code of `proto/store.proto`. `protox` compiles the schema in pure Rust, so building
//! needs no `protoc`.

fn main() {
    let proto = "proto/store.proto";
    println!("cargo:rerun-if-changed={proto}");
    println!("cargo:rerun-if-changed=build.rs");
    let fds = protox::compile([proto], ["proto"]).unwrap_or_else(|e| panic!("cannot compile {proto}: {e}"));
    tonic_prost_build::configure()
        .compile_fds(fds)
        .unwrap_or_else(|e| panic!("cannot generate gRPC code: {e}"));
}
