//! Where the store worker looks for libduckdb at run time on macOS and Linux: run paths baked into the binary,
//! because a signed, notarized macOS app ignores `DYLD_LIBRARY_PATH` (hardened runtime). The library path variable
//! Debrief sets for the worker (debrief-store-client `library_path_env`) stays as a fallback; on Linux it is searched before
//! these. Windows has no run paths: there Debrief puts the library folder on the worker's `PATH`.
//!
//! Shared by `build.rs` (which passes them to the linker) and the crate's tests.

/// The app's `productName` (app/src-tauri/tauri.conf.json): Linux packages install resources into
/// `/usr/lib/<productName>/`.
pub const PRODUCT_NAME: &str = "Debrief";

/// The run paths for a Cargo `target_os`, most specific location first:
///
/// - next to the worker (`@executable_path`, `$ORIGIN`): `cargo run`/dev builds, where build-sidecars.mjs copies
///   the library beside the built worker;
/// - the installed resource folder `lib/` (tauri.sidecars.conf.json): `Debrief.app/Contents/Resources/lib` from
///   `Contents/MacOS/`, and `/usr/lib/Debrief/lib` from `/usr/bin/` in the deb and rpm packages (and the AppImage's
///   `usr/` tree), as Tauri's `resource_dir` resolves them.
///
/// `libduckdb.dylib` is installed as `@rpath/libduckdb.dylib` and `libduckdb.so` has the SONAME `libduckdb.so`
/// (official 1.5.6 release), so both are found through these paths with no `install_name_tool`/`patchelf` step.
pub fn rpaths(target_os: &str) -> Vec<String> {
    match target_os {
        "macos" => vec!["@executable_path".into(), "@executable_path/../Resources/lib".into()],
        "linux" => vec!["$ORIGIN".into(), format!("$ORIGIN/../lib/{PRODUCT_NAME}/lib")],
        _ => Vec::new(),
    }
}

/// The linker arguments for `rpaths`, for `cargo:rustc-link-arg-bins`.
pub fn link_args(target_os: &str) -> Vec<String> {
    rpaths(target_os).into_iter().map(|p| format!("-Wl,-rpath,{p}")).collect()
}
