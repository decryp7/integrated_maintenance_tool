//! The build script's staging of the runtime library (build/runtime_lib.rs).

#[path = "../build/runtime_lib.rs"]
mod runtime_lib;

use std::path::PathBuf;

/// A fresh folder of its own per test (the crate has no tempfile dependency).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("imt-store-duck-runtime-lib-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn names_the_runtime_library_per_target() {
    assert_eq!(runtime_lib::runtime_library("windows"), "duckdb.dll");
    assert_eq!(runtime_lib::runtime_library("macos"), "libduckdb.dylib");
    assert_eq!(runtime_lib::runtime_library("linux"), "libduckdb.so");
}

#[test]
fn stages_the_library_into_the_build_folder_and_replaces_a_stale_one() {
    let tmp = scratch("stage");
    let lib = tmp.join("lib");
    let out = tmp.join("out");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("duckdb.dll"), b"v1").unwrap();
    let dir = runtime_lib::stage(&lib, "duckdb.dll", &out).unwrap().unwrap();
    assert_eq!(dir, out.join("libduckdb"));
    assert_eq!(std::fs::read(dir.join("duckdb.dll")).unwrap(), b"v1");
    // A new library replaces the staged one (a hard link would otherwise keep the old file's contents).
    std::fs::remove_file(lib.join("duckdb.dll")).unwrap();
    std::fs::write(lib.join("duckdb.dll"), b"v2").unwrap();
    runtime_lib::stage(&lib, "duckdb.dll", &out).unwrap();
    assert_eq!(std::fs::read(dir.join("duckdb.dll")).unwrap(), b"v2");
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn a_missing_library_stages_nothing() {
    let tmp = scratch("missing");
    assert_eq!(runtime_lib::stage(&tmp, "duckdb.dll", &tmp.join("out")).unwrap(), None);
    assert!(!tmp.join("out").exists());
    std::fs::remove_dir_all(&tmp).unwrap();
}
