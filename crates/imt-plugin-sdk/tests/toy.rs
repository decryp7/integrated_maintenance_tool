//! The example plugin passes the conformance kit, through the library and through the CLI.

use imt_plugin_sdk::conformance::{run_binary, Options};
use std::path::PathBuf;

fn toy() -> PathBuf {
    // `examples/toy_plugin.rs`, also built as a binary for the tests.
    PathBuf::from(env!("CARGO_BIN_EXE_imt-toy-plugin"))
}

#[test]
fn the_example_plugin_conforms() {
    let report = run_binary(toy(), &[], Options::default());
    assert!(report.passed(), "{report}");
    assert!(report.plugin.starts_with("com.example.toy"));
    assert!(report.checks.iter().any(|c| c.name.contains("transform \"toy.drop-debug\"")));
}

#[test]
fn the_cli_checks_a_plugin_and_reports_usage_errors() {
    let cli = env!("CARGO_BIN_EXE_imt-plugin-check");
    let out = imt_core::process::quiet_command(cli)
        .arg(toy())
        .args([
            "--fuzz",
            "4",
            "--seed",
            "9",
            "--grant",
            "read-opened-files",
            "--grant",
            "network:example.com",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("0 failed"), "{text}");
    for bad in [&["--grant", "root"][..], &["--fuzz"][..], &[][..], &["--help"][..]] {
        let s = imt_core::process::quiet_command(cli).args(bad).output().unwrap();
        assert_eq!(s.status.code(), Some(2), "{bad:?}");
    }
    let s = imt_core::process::quiet_command(cli).arg("/no/such/plugin").output().unwrap();
    assert_eq!(s.status.code(), Some(1));
}
