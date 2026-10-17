//! Generates the gRPC code of the `imt.plugin.v1` contract (`proto/imt/plugin/v1/plugin.proto` at the
//! repository root). `protox` compiles the schema in pure Rust, so building needs no `protoc`. Every
//! `bytes` field becomes `bytes::Bytes`, so record batches and file chunks are never copied by prost.

fn main() {
    let root = "../../proto";
    let proto = "../../proto/imt/plugin/v1/plugin.proto";
    println!("cargo:rerun-if-changed={proto}");
    println!("cargo:rerun-if-changed=build.rs");
    let fds = protox::compile([proto], [root]).unwrap_or_else(|e| panic!("cannot compile {proto}: {e}"));
    tonic_prost_build::configure()
        .bytes(".imt.plugin.v1")
        .compile_fds(fds)
        .unwrap_or_else(|e| panic!("cannot generate gRPC code: {e}"));
}
