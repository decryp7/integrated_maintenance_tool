fn main() {
    // The commit this binary is built from, on every log line and problem report (see `activity::build_commit`).
    // Missing git (a source tarball) is fine: the value is then "unknown".
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
    tauri_build::build()
}
