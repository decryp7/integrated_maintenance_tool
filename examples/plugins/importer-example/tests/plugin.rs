//! The plugin's binary under the test harness: the conformance checks Debrief relies on, and a golden test of the
//! sample (`DEBRIEF_BLESS=1 cargo test` rewrites `tests/golden/`).

use debrief_plugin_test::{Golden, PluginUnderTest};

const PLUGIN: &str = env!("CARGO_BIN_EXE_debrief-example-importer");

#[test]
fn conforms() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    plugin.conformance_with(&["tests/samples/app.log".into()]).assert_passed();
}

#[test]
fn imports_the_sample() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    let out = plugin.import_file("tests/samples/app.log").unwrap();
    assert_eq!(out.records.len(), 3);
    assert_eq!(out.skipped, 1);
    Golden::new("tests/golden").check("app", &out.records);
}
