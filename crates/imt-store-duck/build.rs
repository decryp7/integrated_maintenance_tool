//! Stages the prebuilt libduckdb where `cargo test` / `cargo run` find it at run time (see build/runtime_lib.rs).

#[path = "build/runtime_lib.rs"]
mod runtime_lib;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/runtime_lib.rs");
    println!("cargo:rerun-if-env-changed=DUCKDB_LIB_DIR");
    let (Some(lib_dir), Some(out_dir)) = (std::env::var_os("DUCKDB_LIB_DIR"), std::env::var_os("OUT_DIR")) else {
        return;
    };
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let name = runtime_lib::runtime_library(&target_os);
    let lib_dir = PathBuf::from(lib_dir);
    println!("cargo:rerun-if-changed={}", lib_dir.join(name).display());
    match runtime_lib::stage(&lib_dir, name, &PathBuf::from(out_dir)) {
        Ok(Some(dir)) => println!("cargo:rustc-link-search=native={}", dir.display()),
        Ok(None) => {}
        // Only the convenience is lost: the library can still be put on the library path by hand.
        Err(e) => println!("cargo:warning=could not stage {name} for cargo run/test: {e}"),
    }
}
