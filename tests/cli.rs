//! The `imt` binary's command-line handling.

use std::process::Command;

#[test]
fn missing_path_prints_usage_and_exits_with_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_imt")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage:") && stderr.contains("<path>"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn too_many_arguments_is_a_usage_error_too() {
    let out = Command::new(env!("CARGO_BIN_EXE_imt")).args(["a", "b"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
