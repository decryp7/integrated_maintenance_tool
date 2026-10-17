//! End-to-end checks against the generated files in /samples.

use imt_core::model::Level;
use imt_core::query::{FieldFilter, Query};
use imt_core::workspace::{ChartRequest, FacetRequest, PageRequest, TimelineRequest};
use imt_core::{Profile, Workspace};
use std::path::PathBuf;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Folders of `/samples` that are not logs: `profiling` holds profiler
/// captures (opened by the profiling plugin, tested in `imt-plugin-profiling`).
const NOT_LOGS: &[&str] = &["profiling"];

/// Every entry of `/samples` that the log engine is expected to open.
fn log_samples() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(samples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.file_name().is_some_and(|n| NOT_LOGS.iter().any(|x| n == *x)))
        .collect();
    paths.sort();
    paths
}

fn workspace(profile: Profile) -> Workspace {
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(&log_samples()).unwrap();
    ws
}

fn sample_profile() -> Profile {
    let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
    Profile::from_toml(&text).unwrap()
}

#[test]
fn every_sample_format_is_recognised() {
    let ws = workspace(Profile::default_profile());
    let s = ws.summary();
    let importer = |name: &str| s.sources.iter().find(|x| x.name.ends_with(name)).map(|x| x.importer.as_str());
    assert_eq!(importer("web01/orders-20261014.clef"), Some("json"));
    assert_eq!(importer("gateway/gateway-20261014.json"), Some("json"));
    assert_eq!(importer("worker01/worker-20261014.txt"), Some("text"));
    assert_eq!(importer("billing.log.2026-10-14"), Some("text"));
    assert_eq!(importer("perfmon-export.csv"), Some("csv"));
    assert!(
        s.sources.iter().all(|x| !x.name.contains("profiling")),
        "profiler captures are not log samples"
    );
    for s in &s.sources {
        assert_eq!(s.skipped, 0, "{} skipped lines", s.name);
        assert!(s.from.is_some(), "{} has no timestamps", s.name);
    }
    let unrecognized: Vec<&str> = s.unrecognized.iter().map(|u| u.name.as_str()).collect();
    assert!(unrecognized.iter().any(|n| n.ends_with("registry-export.reg")), "{unrecognized:?}");
    assert!(unrecognized.iter().any(|n| n.ends_with("msinfo32.nfo")), "{unrecognized:?}");
}

#[test]
fn log4net_fields_and_levels() {
    let ws = workspace(Profile::default_profile());
    let q = Query {
        filters: vec![FieldFilter {
            field: "Logger".into(),
            op: "eq".into(),
            value: Some("Billing.Service".into()),
        }],
        ..Default::default()
    };
    let page = ws
        .page(&PageRequest {
            query: q,
            offset: 0,
            limit: 10,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows[0].fields["Thread"], 1);
    let errors = ws
        .records()
        .iter()
        .filter(|r| r.fields.contains_key("Logger") && r.level == Level::Error)
        .count();
    assert!(errors > 0);
    assert!(ws
        .records()
        .iter()
        .any(|r| r.fields.contains_key("Logger") && r.exception.as_deref().is_some_and(|x| x.contains("CommunicationException"))));
}

#[test]
fn timeline_shows_the_crash_and_restart() {
    let ws = workspace(sample_profile());
    let tl = ws
        .timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 400,
            always_show_lifecycle: true,
        })
        .unwrap();
    let lane = |key: &str| {
        tl.lanes
            .iter()
            .find(|l| l.key == key)
            .unwrap_or_else(|| panic!("no lane {key}: {:?}", tl.lanes.iter().map(|l| &l.key).collect::<Vec<_>>()))
    };
    let crashed = lane("WEB02 · 4120");
    assert!(crashed.markers.iter().any(|m| m.kind == "fatal"));
    assert!(crashed.markers.iter().any(|m| m.kind == "start"));
    assert!(!crashed.markers.iter().any(|m| m.kind == "stop"));
    let restarted = lane("WEB02 · 5532");
    assert!(restarted.first > crashed.last);
    assert!(lane("WORKER01 · 2210").markers.iter().any(|m| m.kind == "stop"));

    // Lifecycle markers survive a level filter.
    let errors_only = Query {
        levels: vec![Level::Error],
        ..Default::default()
    };
    let tl = ws
        .timeline(&TimelineRequest {
            query: errors_only,
            buckets: 400,
            always_show_lifecycle: true,
        })
        .unwrap();
    let crashed = tl.lanes.iter().find(|l| l.key == "WEB02 · 4120").unwrap();
    assert!(crashed.markers.iter().any(|m| m.kind == "start"));
    assert!(crashed.matched < crashed.total);
}

#[test]
fn extracted_metrics_chart() {
    let ws = workspace(sample_profile());
    let s = ws.summary();
    let cpu = s.fields.iter().find(|f| f.name == "CpuPercent").unwrap();
    assert!(cpu.numeric && cpu.derived);
    let chart = ws
        .chart(&ChartRequest {
            query: Query::default(),
            chart_id: Some("cpu".into()),
            def: None,
            buckets: 200,
        })
        .unwrap();
    assert_eq!(chart.bucket_ms, 60_000);
    let keys: Vec<&str> = chart.series.iter().map(|s| s.key.as_str()).collect();
    assert!(keys.contains(&"WORKER01 · 2208"), "{keys:?}");
    let web02 = chart.series.iter().find(|s| s.key == "WEB02 · 4120").unwrap();
    let peak = web02.points.iter().filter_map(|p| p.1).fold(0.0, f64::max);
    assert!(peak > 90.0, "peak {peak}");
    assert!(chart.lifecycle.iter().any(|l| l.kind == "fatal"));
}

#[test]
fn paging_with_anchor_and_facets() {
    let ws = workspace(Profile::default_profile());
    let fatal = ws.records().iter().find(|r| r.level == Level::Fatal).unwrap().id;
    let page = ws
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 100,
            descending: true,
            anchor: Some(fatal),
        })
        .unwrap();
    let idx = page.anchor_index.unwrap();
    assert_eq!(page.rows[idx - page.offset].id, fatal);

    let facet = ws
        .facet(&FacetRequest {
            query: Query::default(),
            field: "MachineName".into(),
            limit: 10,
        })
        .unwrap();
    let names: Vec<&str> = facet.values.iter().map(|v| v.value.as_str()).collect();
    for m in ["WEB01", "WEB02", "WORKER01", "GATEWAY"] {
        assert!(names.contains(&m), "{names:?}");
    }
}
