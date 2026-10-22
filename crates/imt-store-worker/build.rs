//! Bakes the run paths of libduckdb into the store worker binary on macOS and Linux (see build/rpath.rs).

#[path = "build/rpath.rs"]
mod rpath;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/rpath.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    for arg in rpath::link_args(&target_os) {
        println!("cargo:rustc-link-arg-bins={arg}");
    }
}
