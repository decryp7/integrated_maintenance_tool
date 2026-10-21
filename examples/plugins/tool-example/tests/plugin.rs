//! The plugin's binary under the test harness: conformance, and the tool's operations.

use imt_plugin_test::PluginUnderTest;
use serde_json::json;

const PLUGIN: &str = env!("CARGO_BIN_EXE_imt-example-tool");

#[test]
fn conforms() {
    PluginUnderTest::launch(PLUGIN).unwrap().conformance().assert_passed();
}

#[test]
fn converts_seconds_and_milliseconds() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    let s = plugin.invoke("epoch", "convert", json!({ "value": 1_792_576_800 })).unwrap();
    assert_eq!(s["utc"], "2026-10-21T10:00:00.000Z");
    let ms = plugin.invoke("epoch", "convert", json!({ "value": 1_792_576_800_123i64 })).unwrap();
    assert_eq!(ms["utc"], "2026-10-21T10:00:00.123Z");
    assert!(plugin.invoke("epoch", "now", json!({})).unwrap()["ms"].as_i64().unwrap() > 0);
    // Arguments that do not match the schema are refused before the tool runs.
    let e = plugin.invoke("epoch", "convert", json!({ "value": "soon" })).unwrap_err();
    assert!(!e.message.is_empty());
}
