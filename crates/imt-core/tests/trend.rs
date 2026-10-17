//! MemoryGrowth detection on synthetic series and on /samples.

use imt_core::analysis::{detect, Anomaly, AnomalyConfig, AnomalyKind};
use imt_core::model::{Level, Record};
use imt_core::query::group_key;
use imt_core::{Profile, Workspace};
use serde_json::json;
use std::path::PathBuf;

const T0: i64 = 1_791_968_400_000;
const MIN: i64 = 60_000;
const MB: f64 = 1_048_576.0;

fn field_rec(id: u32, ts: i64, field: &str, v: f64) -> Record {
    let mut r = Record::new(id);
    r.id = id;
    r.ts = Some(ts);
    r.level = Level::Information;
    r.fields.insert(field.into(), json!(v));
    r
}

fn counter_rec(id: u32, ts: i64, machine: &str, process: &str, counter: &str, v: f64) -> Record {
    let mut r = Record::new(id);
    r.id = id;
    r.ts = Some(ts);
    for (k, val) in [("MachineName", machine), ("ProcessName", process), ("Counter", counter)] {
        r.fields.insert(k.into(), json!(val));
    }
    r.fields.insert("Value".into(), json!(v));
    r
}

fn growth(recs: &[(Record, &str)]) -> Vec<Anomaly> {
    detect(recs.iter().map(|(r, k)| (r, *k)), &AnomalyConfig::default())
        .into_iter()
        .filter(|a| a.kind == AnomalyKind::MemoryGrowth)
        .collect()
}

fn series(key: &'static str, field: &str, n: i64, step: i64, f: impl Fn(i64) -> f64) -> Vec<(Record, &'static str)> {
    (0..n).map(|i| (field_rec(i as u32, T0 + i * step, field, f(i)), key)).collect()
}

#[test]
fn leak_is_detected() {
    // 600 MB + 18 MB/min for 100 minutes, sampled every minute.
    let recs = series("WEB02 · 4120", "WorkingSetMb", 101, MIN, |i| {
        600.0 + 18.0 * i as f64 + (i % 3) as f64
    });
    let found = growth(&recs);
    assert_eq!(found.len(), 1, "{found:#?}");
    let a = &found[0];
    assert_eq!(a.instance.as_deref(), Some("WEB02 · 4120"));
    assert!(
        a.message
            .starts_with("WorkingSetMb on WEB02 · 4120 grew 1.8 GB over 1 h 40 min (~18 MB/min, R² 1.00)"),
        "{}",
        a.message
    );
    assert_eq!((a.from, a.to), (T0, T0 + 100 * MIN));
    assert!(a.score > 1.0 && !a.sample_ids.is_empty() && a.sample_ids.len() <= 5);
    assert_eq!(*a.sample_ids.last().unwrap(), 100);
}

#[test]
fn sawtooth_noise_short_and_small_are_ignored() {
    // GC sawtooth around a flat level.
    let saw = series("A", "PrivateMb", 120, MIN, |i| 500.0 + (i % 20) as f64 * 10.0);
    assert!(growth(&saw).is_empty());
    // Noisy flat.
    let noisy = series("A", "PrivateMb", 120, MIN, |i| 500.0 + ((i * 7919) % 13) as f64);
    assert!(growth(&noisy).is_empty());
    // Too short a span, too few points, constant, shrinking.
    assert!(growth(&series("A", "PrivateMb", 8, MIN, |i| 100.0 + 50.0 * i as f64)).is_empty());
    assert!(growth(&series("A", "PrivateMb", 4, 5 * MIN, |i| 100.0 + 50.0 * i as f64)).is_empty());
    assert!(growth(&series("A", "PrivateMb", 60, MIN, |_| 300.0)).is_empty());
    assert!(growth(&series("A", "PrivateMb", 60, MIN, |i| 3000.0 - 10.0 * i as f64)).is_empty());
    // Growth under the absolute floor (30 MB) or under 20 % of a big start.
    assert!(growth(&series("A", "PrivateMb", 60, MIN, |i| 100.0 + 0.5 * i as f64)).is_empty());
    assert!(growth(&series("A", "PrivateMb", 60, MIN, |i| 10_000.0 + 2.0 * i as f64)).is_empty());
    // Unknown field.
    assert!(growth(&series("A", "Other", 60, MIN, |i| 100.0 + 50.0 * i as f64)).is_empty());
}

#[test]
fn handles_use_a_count_floor() {
    let leak = series("A", "HandleCount", 60, MIN, |i| 1000.0 + 20.0 * i as f64);
    let found = growth(&leak);
    assert_eq!(found.len(), 1);
    assert!(
        found[0]
            .message
            .starts_with("HandleCount on A grew 1180 over 59 min 0 s (~20/min, R² 1.00)"),
        "{}",
        found[0].message
    );
    // 59 * 5 = 295 handles: below the floor.
    assert!(growth(&series("A", "Handles", 60, MIN, |i| 1000.0 + 5.0 * i as f64)).is_empty());
    // Zero start: score is relative to the floor.
    let from_zero = growth(&series("A", "Handles", 60, MIN, |i| 20.0 * i as f64));
    assert_eq!(from_zero.len(), 1);
    assert!(from_zero[0].score > 1.0);
}

#[test]
fn perfmon_counters_group_by_machine_and_process() {
    let mut recs: Vec<(Record, &str)> = Vec::new();
    for i in 0..90i64 {
        let ts = T0 + i * MIN;
        let id = (i * 10) as u32;
        // Leaking process in bytes, flat twin on another machine, plain counter ignored.
        recs.push((
            counter_rec(id, ts, "WEB02", "OrderService", "Private Bytes", 500.0 * MB + 20.0 * MB * i as f64),
            "x",
        ));
        recs.push((counter_rec(id + 1, ts, "WEB01", "OrderService", "Private Bytes", 500.0 * MB), "x"));
        recs.push((
            counter_rec(id + 2, ts, "WEB02", "OrderService", "% Processor Time", 10.0 + i as f64),
            "x",
        ));
        recs.push((
            counter_rec(id + 3, ts, "WEB02", "OrderService", "Handle Count", 900.0 + 15.0 * i as f64),
            "x",
        ));
        let mut r = counter_rec(
            id + 4,
            ts,
            "WEB02",
            "OrderService",
            "Private Bytes",
            100.0 * MB + 30.0 * MB * i as f64,
        );
        r.fields.insert("InstanceIndex".into(), json!(2));
        recs.push((r, "x"));
        // Records without the needed fields are skipped.
        let mut bare = Record::new(1);
        bare.ts = Some(ts);
        recs.push((bare, "x"));
        let mut text = counter_rec(id + 5, ts, "WEB02", "svc", "Private Bytes", 1.0);
        text.fields.insert("Value".into(), json!("n/a"));
        text.fields.remove("ProcessName");
        recs.push((text, "x"));
    }
    let found = growth(&recs);
    let msgs: Vec<&str> = found.iter().map(|a| a.message.as_str()).collect();
    assert_eq!(found.len(), 3, "{msgs:#?}");
    assert!(
        msgs.iter()
            .any(|m| m.starts_with("Handle Count on WEB02 · OrderService grew 1335 over 1 h 29 min")),
        "{msgs:#?}"
    );
    assert!(msgs
        .iter()
        .any(|m| m.starts_with("Private Bytes on WEB02 · OrderService grew 1.7 GB over 1 h 29 min (~20 MB/min, R² 1.00)")));
    assert!(msgs
        .iter()
        .any(|m| m.starts_with("Private Bytes on WEB02 · OrderService#2 grew 2.6 GB")));
}

#[test]
fn serialises_as_memory_growth() {
    assert_eq!(serde_json::to_string(&AnomalyKind::MemoryGrowth).unwrap(), "\"memoryGrowth\"");
    let cfg: AnomalyConfig = serde_json::from_str(r#"{"growth":{"minR2":0.5}}"#).unwrap();
    assert_eq!(cfg.growth.min_r2, 0.5);
    assert_eq!(cfg.growth.min_duration, 600_000);
}

#[test]
fn leak_in_the_generated_samples() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[dir]).unwrap();
    let summary = ws.summary();
    let names: std::collections::HashMap<u32, &str> = summary.sources.iter().map(|s| (s.id, s.name.as_str())).collect();
    let group_by = summary.default_group_by.clone();
    let keys: Vec<String> = ws
        .records()
        .iter()
        .map(|r| group_key(&group_by, r, names.get(&r.source).copied().unwrap_or("")))
        .collect();
    let found = detect(ws.records().iter().zip(keys.iter().map(String::as_str)), &AnomalyConfig::default());
    let leaks: Vec<&Anomaly> = found.iter().filter(|a| a.kind == AnomalyKind::MemoryGrowth).collect();
    assert!(
        leaks
            .iter()
            .any(|a| a.instance.as_deref().is_some_and(|i| i.contains("WEB02") && i.contains("4120"))
                && a.message.starts_with("WorkingSetMb on")),
        "{leaks:#?}"
    );
    assert!(
        leaks.iter().all(|a| !a.instance.as_deref().unwrap_or("").contains("WEB01")),
        "{leaks:#?}"
    );
}
