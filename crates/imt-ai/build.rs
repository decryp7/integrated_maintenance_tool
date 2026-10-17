//! Generates the gRPC client and server code for the inference worker from `imt-ai-worker/proto/llm.proto`.
//! `protox` compiles the schema in pure Rust, so building needs no `protoc`.

fn main() {
    let proto = "../imt-ai-worker/proto/llm.proto";
    println!("cargo:rerun-if-changed={proto}");
    println!("cargo:rerun-if-changed=build.rs");
    let fds = protox::compile([proto], ["../imt-ai-worker/proto"]).unwrap_or_else(|e| panic!("cannot compile {proto}: {e}"));
    tonic_prost_build::configure()
        .compile_fds(fds)
        .unwrap_or_else(|e| panic!("cannot generate gRPC code: {e}"));
}
