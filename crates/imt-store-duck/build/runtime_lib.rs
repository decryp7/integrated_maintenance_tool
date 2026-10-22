//! Makes `cargo test` / `cargo run` find the prebuilt libduckdb at run time, with no `PATH` or `LD_LIBRARY_PATH`
//! set by hand (e.g. after `cargo xtask libduckdb`).
//!
//! Cargo puts the linker search paths of build scripts on the library path of what it runs, but only those inside
//! `target/<profile>/`; `DUCKDB_LIB_DIR` (a cache folder) is not. So the build script links (or copies) the
//! runtime library from `DUCKDB_LIB_DIR` into its `OUT_DIR`, which is inside, and names that folder as a search
//! path. That works for every profile and target folder and survives `cargo clean`. Shipped builds do not depend on
//! it: the installers bundle the library as a resource (app/scripts/build-sidecars.mjs).

use std::path::{Path, PathBuf};

/// The file name of the runtime library for a Cargo `target_os`.
pub fn runtime_library(target_os: &str) -> &'static str {
    match target_os {
        "windows" => "duckdb.dll",
        "macos" | "ios" => "libduckdb.dylib",
        _ => "libduckdb.so",
    }
}

/// Puts `lib_dir/<name>` into `out_dir` (a hard link when possible, else a copy) and returns the folder to name as
/// a search path, or `None` when the library is not in `lib_dir` (libduckdb-sys then reports what is missing).
pub fn stage(lib_dir: &Path, name: &str, out_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let src = lib_dir.join(name);
    if !src.is_file() {
        return Ok(None);
    }
    let dir = out_dir.join("libduckdb");
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(name);
    if dst.exists() {
        std::fs::remove_file(&dst)?;
    }
    if std::fs::hard_link(&src, &dst).is_err() {
        std::fs::copy(&src, &dst)?;
    }
    Ok(Some(dir))
}
