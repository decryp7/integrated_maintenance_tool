//! The plugin's binary under the test harness: conformance, and the transform over a few records.

use imt_plugin_api::Record;
use imt_plugin_test::PluginUnderTest;

const PLUGIN: &str = env!("CARGO_BIN_EXE_imt-example-transform");

fn record(message: &str, user: &str) -> Record {
    let mut r = Record::new(1);
    r.message = message.into();
    r.fields.insert("User".into(), user.into());
    r
}

#[test]
fn conforms() {
    PluginUnderTest::launch(PLUGIN).unwrap().conformance().assert_passed();
}

#[test]
fn redacts_addresses_and_drops_empty_records() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    let out = plugin
        .transform(
            "redact-emails",
            "a.log",
            &[record("login by ann@example.com ok", "ann@example.com"), record("  ", "bob")],
        )
        .unwrap();
    assert_eq!(out.keep, [true, false]);
    let first = out.rows[0].clone().into_record();
    assert_eq!(first.message, "login by <email> ok");
    assert_eq!(first.fields["User"], "<email>");
}
