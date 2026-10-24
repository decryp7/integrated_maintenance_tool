//! PerfMon support end to end: the fixture through a workspace, the profile and the leak detector.

use crate::common;

use debrief_core::analysis::{detect, AnomalyConfig, AnomalyKind};
use debrief_core::model::Record;
use debrief_core::{Profile, Workspace};
use std::path::Path;

fn dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../debrief-plugin-windows")
}

fn profile() -> Profile {
    let toml = std::fs::read_to_string(dir().join("profiles/windows-performance.debriefprofile.toml")).unwrap();
    Profile::from_toml(&toml).unwrap()
}

fn workspace() -> Workspace {
    let registry = common::registry(&[common::WINDOWS]);
    let mut ws = Workspace::with_registry(profile(), registry).unwrap();
    ws.open(&[dir().join("tests/fixtures/perfmon.csv")]).unwrap();
    ws
}

#[test]
fn fixture_is_read_by_the_perfmon_importer_not_generic_csv() {
    let ws = workspace();
    let s = ws.summary();
    assert_eq!(s.sources.len(), 1);
    assert_eq!(s.sources[0].importer, "perfmon-csv");
    // 45 rows x 14 counters, minus the blanked samples.
    assert_eq!(ws.records().len(), 45 * 14 - 3);
    let leak: &Record = ws
        .records()
        .iter()
        .find(|r| r.fields["MachineName"] == "WEB02" && r.fields["Counter"] == "Private Bytes" && r.fields["ProcessName"] == "OrderService")
        .unwrap();
    assert_eq!(leak.fields["ProcessId"], 5532);
    assert_eq!(leak.fields["InstanceIndex"], 1);
    assert_eq!(leak.ts, Some(1_791_968_400_123 - 2 * 3_600_000));
}

#[test]
fn profile_charts_load_and_the_leak_is_found() {
    let p = profile();
    let titles: Vec<&str> = p.chart.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Private Bytes by process",
            "CPU % by process",
            "Available MBytes by machine",
            "Handle Count by process"
        ]
    );
    let ws = workspace();
    let keys: Vec<String> = ws.records().iter().map(|_| "x".to_string()).collect();
    let found = detect(ws.records().iter().zip(keys.iter().map(String::as_str)), &AnomalyConfig::default());
    let leaks: Vec<&str> = found
        .iter()
        .filter(|a| a.kind == AnomalyKind::MemoryGrowth)
        .map(|a| a.message.as_str())
        .collect();
    assert!(
        leaks.iter().any(|m| m.starts_with("Private Bytes on WEB02 · OrderService#1 grew ")),
        "{leaks:#?}"
    );
    assert!(
        leaks.iter().any(|m| m.starts_with("Handle Count on WEB02 · OrderService#1 grew ")),
        "{leaks:#?}"
    );
    assert!(leaks.iter().all(|m| !m.contains("WEB01")), "{leaks:#?}");
}
