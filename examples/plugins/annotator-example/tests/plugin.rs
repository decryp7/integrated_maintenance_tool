//! The plugin's binary under the test harness: conformance, and the bursts it finds in a view of records.

use imt_plugin_api::{Level, Record};
use imt_plugin_test::PluginUnderTest;

const PLUGIN: &str = env!("CARGO_BIN_EXE_imt-example-annotator");

fn event(ts: i64, level: Level, instance: &str) -> Record {
    let mut r = Record::new(1);
    r.ts = Some(ts);
    r.level = level;
    r.fields.insert("Instance".into(), instance.into());
    r
}

#[test]
fn conforms() {
    PluginUnderTest::launch(PLUGIN).unwrap().conformance().assert_passed();
}

#[test]
fn marks_bursts_per_instance() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    let records = vec![
        event(0, Level::Error, "web01"),
        event(10_000, Level::Error, "web01"),
        event(20_000, Level::Fatal, "web01"),
        event(25_000, Level::Information, "web01"),
        // Too far apart on web02.
        event(0, Level::Error, "web02"),
        event(200_000, Level::Error, "web02"),
        event(400_000, Level::Error, "web02"),
    ];
    let a = plugin.annotate("error-bursts", &records).unwrap();
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(
        (a[0].from, a[0].to, a[0].instance.as_deref(), a[0].label.as_str()),
        (0, 20_000, Some("web01"), "3 errors")
    );
}
