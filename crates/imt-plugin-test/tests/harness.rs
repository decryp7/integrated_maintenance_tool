//! The harness against its misbehaving fixture: conformance passes for a well-behaved plugin, a panic is a typed
//! error and not a hang, a hang is cut off, and a contained plugin cannot read the user's files.

use imt_ipc::peer::code;
use imt_plugin_test::{Launch, PluginUnderTest};
use std::time::{Duration, Instant};

const FIXTURE: &str = env!("CARGO_BIN_EXE_imt-plugin-test-fixture");

#[test]
fn a_well_behaved_plugin_passes_conformance_and_imports() {
    let p = PluginUnderTest::launch(FIXTURE).unwrap();
    assert_eq!(p.manifest().id.as_ref(), "net.decryptology.test-fixture");
    let report = p.conformance();
    report.assert_passed();
    assert_eq!(report.checks.len(), 6, "{report}");
    let out = p.import("fixture", "a.log", b"one\ntwo\n").unwrap();
    assert_eq!(out.records.iter().map(|r| r.message.as_str()).collect::<Vec<_>>(), ["one", "two"]);
    assert_eq!(p.sniff("fixture", "a.log", b"#fixture\n").unwrap(), 100);
    assert_eq!(p.invoke("fixture.tool", "echo", serde_json::json!({"a": 1})).unwrap()["a"], 1);
    assert!(p.invoke("fixture.tool", "nope", serde_json::json!({})).is_err());
}

#[test]
fn a_panic_is_a_typed_error_and_the_plugin_keeps_serving() {
    let p = PluginUnderTest::launch(FIXTURE).unwrap();
    let e = p.import("fixture", "a.log", b"panic\n").unwrap_err();
    assert!(e.to_string().contains("panicked"), "{e}");
    let raw = p.request_raw(imt_ipc::wire::encode(&imt_plugin_api::protocol::PluginRequest::Manifest).unwrap());
    assert!(raw.is_ok() && p.alive());
    let e = p.import("fixture", "a.log", b"fail\n").unwrap_err();
    assert!(e.to_string().contains("on purpose"), "{e}");
    match p.request_raw(vec![0xff; 4]) {
        Err(imt_ipc::IpcError::Remote(e)) => assert_eq!(e.code, code::INVALID),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_hung_import_is_cut_off_and_fails_the_cancellation_check() {
    let p = PluginUnderTest::launch(FIXTURE).unwrap();
    let started = Instant::now();
    let e = p.import_within("fixture", "a.log", b"hang\n", Duration::from_secs(1)).unwrap_err();
    assert!(e.to_string().contains("did not answer"), "{e}");
    assert!(started.elapsed() < Duration::from_secs(10));
    let hang = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(hang.path(), b"hang\n").unwrap();
    let fresh = PluginUnderTest::launch(FIXTURE).unwrap();
    let cancel = fresh
        .conformance_with(&[hang.path().to_path_buf()])
        .checks
        .into_iter()
        .find(|c| c.name == "cancellation")
        .unwrap();
    // The bounded-output check before it would wait for the hang too; cancellation is what is asserted.
    assert!(!cancel.passed, "{cancel:?}");
}

#[cfg(windows)]
#[test]
fn a_contained_plugin_cannot_read_the_users_files_unless_run_as_first_party() {
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), b"secret").unwrap();
    let args = serde_json::json!({ "path": secret.path() });
    let contained = PluginUnderTest::launch(FIXTURE).unwrap();
    let r = contained.invoke("fixture.tool", "read", args.clone()).unwrap();
    assert!(r.get("denied").is_some(), "{r}");
    // Its own data folder is its to use.
    std::fs::write(contained.data_dir().join("mine.txt"), b"ok").unwrap();
    let own = serde_json::json!({ "path": contained.data_dir().join("mine.txt") });
    assert_eq!(contained.invoke("fixture.tool", "read", own).unwrap()["read"], 2);
    let first_party = PluginUnderTest::launch_with(
        FIXTURE,
        Launch {
            third_party: false,
            ..Launch::default()
        },
    )
    .unwrap();
    // IMT's own plugins run at low integrity, which reads but cannot write.
    assert_eq!(first_party.invoke("fixture.tool", "read", args).unwrap()["read"], 6);
    assert_eq!(contained.granted(), [imt_plugin_api::Permission::Network]);
}
