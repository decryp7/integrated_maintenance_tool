//! The plugin's binary under the test harness: conformance, a run of the collector with its task reports, its
//! output in the private data folder, and stopping it.

use debrief_plugin_test::{PluginUnderTest, TaskReport};
use serde_json::json;
use std::time::{Duration, Instant};

const PLUGIN: &str = env!("CARGO_BIN_EXE_debrief-example-collector");

fn wait_until(mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(20), "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn finished(p: &PluginUnderTest) -> bool {
    p.task_reports().iter().any(|r| matches!(r, TaskReport::Finished { .. }))
}

#[test]
fn conforms() {
    PluginUnderTest::launch(PLUGIN).unwrap().conformance().assert_passed();
}

#[test]
fn collects_into_its_data_folder_and_reports_its_task() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    plugin
        .invoke("heartbeat", "start", json!({ "count": 3, "intervalMs": 10 }))
        .unwrap();
    wait_until(|| finished(&plugin));
    let s = plugin.invoke("heartbeat", "samples", json!({})).unwrap();
    assert_eq!(s["samples"].as_array().unwrap().len(), 3);
    let csv = std::fs::read_to_string(plugin.data_dir().join("heartbeats.csv")).unwrap();
    assert_eq!(csv.lines().count(), 3);
    assert!(matches!(&plugin.task_reports()[0], TaskReport::Started { task, .. } if task.stoppable));
}

#[test]
fn stopping_ends_the_task_early() {
    let plugin = PluginUnderTest::launch(PLUGIN).unwrap();
    plugin
        .invoke("heartbeat", "start", json!({ "count": 10_000, "intervalMs": 10 }))
        .unwrap();
    assert!(
        plugin
            .invoke("heartbeat", "start", json!({ "count": 1, "intervalMs": 10 }))
            .is_err(),
        "one run at a time"
    );
    plugin.stop_task("heartbeat", "heartbeat").unwrap();
    wait_until(|| finished(&plugin));
    let n = plugin.invoke("heartbeat", "samples", json!({})).unwrap()["samples"]
        .as_array()
        .unwrap()
        .len();
    assert!(n < 10_000);
}
