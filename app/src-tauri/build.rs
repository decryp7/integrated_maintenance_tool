fn main() {
    // The commit this binary is built from, on every log line and problem report (see `activity::build_commit`).
    // Missing git (a source tarball) is fine: the value is then "unknown".
    // A build script runs no console windows on users' machines; the quiet spawner is not needed here.
    #[allow(clippy::disallowed_methods)]
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=IMT_BUILD_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    // Tauri embeds its app manifest into the app binary only; test binaries then fail to load on Windows
    // (STATUS_ENTRYPOINT_NOT_FOUND, Common Controls v6 missing). Embed the same manifest into every binary
    // this crate links (`rustc-link-arg` covers tests too) and tell tauri-build not to add its own.
    let msvc =
        std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut windows = tauri_build::WindowsAttributes::new();
    if msvc {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    }
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)) {
        panic!("tauri-build failed: {e:#}");
    }
}
