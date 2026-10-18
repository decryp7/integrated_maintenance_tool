//! Snapshot tests of the built-in importers against `/samples`.
//!
//! Each log sample is imported through the importer the registry selects for
//! it (decode, sniff, import, as the workspace does), and a compact, deterministic projection of the result (counts plus the
//! first records with sorted fields) is compared with the committed snapshot in
//! `tests/snapshots/`. After an intended change to parsing, review and accept
//! the new output with `cargo insta review` (or `INSTA_UPDATE=always cargo test`).

use chrono::{DateTime, NaiveDate, SecondsFormat};
use imt_core::importers::{decode, ImportCtx, Importers};
use imt_core::model::{Level, Record};
use imt_core::profile::CompiledProfile;
use imt_core::Profile;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Records shown in full per sample; the rest are only counted.
const FIRST_RECORDS: usize = 5;
/// Exceptions are cut to their first lines so snapshots stay compact.
const EXCEPTION_LINES: usize = 3;
/// Fixed "file modified" date: logs that carry only a time of day must not depend on the checkout's mtime.
const FILE_DATE: (i32, u32, u32) = (2026, 10, 14);

/// Every `/samples` log the built-in importers read: (snapshot name, path under `/samples`, expected importer).
/// `log4j-xml` is read by the formats plugin; `windows`, `web`, `linux` and `otel` by their plugins, which
/// snapshot them in their own `tests/samples.rs`; `profiling`, `configs` and `share` are not logs; archive members
/// (`archives/`) are checked in `tests/samples.rs`.
const SAMPLES: &[(&str, &str, &str)] = &[
    ("csv-perfmon", "csv/perfmon-export.csv", "csv"),
    (
        "docker-json",
        "docker/containers/f4a5f340c2fdab20873c795900a5e37f5f324094d80e94c0e337ae2c8ee98edf/f4a5f340c2fdab20873c795900a5e37f5f324094d80e94c0e337ae2c8ee98edf-json.log",
        "json",
    ),
    (
        "kubernetes-cri-0",
        "kubernetes/pods/shop_order-events-7c9d8b5f4-x2k4p_1d2ee59b-03c2-8ba5-2786-5799c6b578de/consumer/0.log",
        "text",
    ),
    (
        "kubernetes-cri-1",
        "kubernetes/pods/shop_order-events-7c9d8b5f4-x2k4p_1d2ee59b-03c2-8ba5-2786-5799c6b578de/consumer/1.log",
        "text",
    ),
    ("clock-skew-gateway", "clock-skew/gateway/requests-20261014.clef", "json"),
    ("clock-skew-web02", "clock-skew/web02/requests-20261014.clef", "json"),
    ("log4net", "log4net/legacy-billing/billing.log.2026-10-14", "text"),
    ("monitor-bill01", "monitor/BILL01/process-monitor.clef", "json"),
    ("runs-batch01", "runs/batch01/report-service-20261014.txt", "text"),
    ("runs-shared", "runs/shared/workers-20261014.log", "text"),
    ("serilog-clef-web01", "serilog-clef/web01/orders-20261014.clef", "json"),
    ("serilog-clef-web02", "serilog-clef/web02/orders-20261014.clef", "json"),
    ("serilog-json-gateway", "serilog-json/gateway/gateway-20261014.json", "json"),
    ("serilog-text-worker01", "serilog-text/worker01/worker-20261014.txt", "text"),
    ("sqlserver-errorlog-upper", "sqlserver/SQL01/ERRORLOG", "text"),
    ("sqlserver-errorlog-lower", "sqlserver/sqlrpt01/errorlog", "text"),
];

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

#[derive(Serialize)]
struct Snapshot {
    sample: String,
    importer: &'static str,
    records: usize,
    skipped: usize,
    /// Records per level, in severity order; levels without records are left out.
    levels: BTreeMap<String, usize>,
    with_timestamp: usize,
    first_timestamp: Option<String>,
    last_timestamp: Option<String>,
    /// Every field name seen in any record.
    fields: BTreeSet<String>,
    first: Vec<RecordSnapshot>,
}

#[derive(Serialize)]
struct RecordSnapshot {
    line: u32,
    timestamp: Option<String>,
    level: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exception: Option<String>,
    fields: Value,
}

fn iso(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms).map_or_else(|| format!("invalid({ms})"), |t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The value with every object's keys sorted, recursively, so the snapshot never depends on map order.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> = map.iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect::<Map<_, _>>())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn record_snapshot(r: &Record) -> RecordSnapshot {
    RecordSnapshot {
        line: r.line,
        timestamp: r.ts.map(iso),
        level: r.level.as_str(),
        message: r.message.clone(),
        template: r.template.clone(),
        exception: r
            .exception
            .as_ref()
            .map(|e| e.lines().take(EXCEPTION_LINES).collect::<Vec<_>>().join("\n")),
        fields: canonical(&Value::Object(r.fields.clone())),
    }
}

/// Imports one sample the way the workspace does (decode, sniff, import) and projects the result.
fn snapshot_of(rel: &str, profile: &CompiledProfile, importers: &Importers) -> Snapshot {
    let bytes = std::fs::read(samples().join(rel)).unwrap();
    let content = decode(&bytes).unwrap_or_else(|| panic!("{rel} does not decode as text"));
    let (y, m, d) = FILE_DATE;
    let ctx = ImportCtx {
        profile,
        path: Path::new(rel),
        file_date: NaiveDate::from_ymd_opt(y, m, d),
    };
    let head: String = content.chars().take(64 * 1024).collect();
    let importer = importers
        .select(&ctx, &head)
        .unwrap_or_else(|| panic!("no importer recognises {rel}"));
    let imported = importer.import(&ctx, &content).unwrap_or_else(|e| panic!("{rel}: {e:#}"));
    let records = &imported.records;

    let mut levels = BTreeMap::new();
    for level in Level::ALL {
        let n = records.iter().filter(|r| r.level == level).count();
        if n > 0 {
            levels.insert(format!("{}-{}", level.index(), level.as_str()), n);
        }
    }
    let stamps: Vec<i64> = records.iter().filter_map(|r| r.ts).collect();
    Snapshot {
        sample: rel.to_string(),
        importer: importer.id(),
        records: records.len(),
        skipped: imported.skipped,
        levels,
        with_timestamp: stamps.len(),
        first_timestamp: stamps.iter().min().copied().map(iso),
        last_timestamp: stamps.iter().max().copied().map(iso),
        fields: records.iter().flat_map(|r| r.fields.keys().cloned()).collect(),
        first: records.iter().take(FIRST_RECORDS).map(record_snapshot).collect(),
    }
}

#[test]
fn every_sample_matches_its_importer_snapshot() {
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let importers = Importers::new();
    let mut used = BTreeSet::new();
    for &(name, rel, expected) in SAMPLES {
        let snap = snapshot_of(rel, &profile, &importers);
        assert_eq!(snap.importer, expected, "{rel} was read by the wrong importer");
        assert!(snap.records > 0, "{rel} produced no records");
        used.insert(snap.importer);
        insta::assert_yaml_snapshot!(name, snap);
    }
    let builtin: BTreeSet<&str> = importers.text_importers().map(|i| i.id()).collect();
    assert_eq!(used, builtin, "every built-in importer must be covered by at least one sample");
}

#[test]
fn snapshot_projection_is_deterministic() {
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let importers = Importers::new();
    for &(_, rel, _) in SAMPLES {
        let a = serde_json::to_string(&snapshot_of(rel, &profile, &importers)).unwrap();
        let b = serde_json::to_string(&snapshot_of(rel, &profile, &importers)).unwrap();
        assert_eq!(a, b, "{rel} imports differently on a second run");
    }
}

#[test]
fn every_sample_file_exists_and_names_are_unique() {
    let names: BTreeSet<&str> = SAMPLES.iter().map(|s| s.0).collect();
    assert_eq!(names.len(), SAMPLES.len(), "duplicate snapshot name");
    for &(_, rel, _) in SAMPLES {
        assert!(
            samples().join(rel).is_file(),
            "{rel} is missing; regenerate with samples/generate.py"
        );
    }
}

#[test]
fn canonical_sorts_nested_keys() {
    let v: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[{"z":1,"y":2}],"c":2}}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&canonical(&v)).unwrap(),
        r#"{"a":{"c":2,"d":[{"y":2,"z":1}]},"b":1}"#
    );
    assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
}
