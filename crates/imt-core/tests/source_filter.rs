//! `Query::sources`: restricting every query to some source ids, so two files
//! with the same name in different folders (two monitor runs) never mix.

use imt_core::api::{ChartRequest, ExportRequest, FacetRequest, FieldStatsRequest, PageRequest, QueryRequest, TimelineRequest};
use imt_core::query::Query;
use imt_core::{Profile, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A temporary folder removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("imt-sources-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `run1/monitor.clef` with 3 events and `run2/monitor.clef` with 5.
fn two_runs() -> (TempDir, Workspace) {
    let d = TempDir::new();
    for (run, n) in [("run1", 3), ("run2", 5)] {
        let dir = d.0.join(run);
        std::fs::create_dir_all(&dir).unwrap();
        let text: String = (1..=n)
            .map(|i| format!(r#"{{"@t":"2026-10-14T10:00:0{i}Z","@l":"Warning","@mt":"{run} sample {i}","Run":"{run}","Cpu":{i}}}"#) + "\n")
            .collect();
        std::fs::write(dir.join("monitor.clef"), text).unwrap();
    }
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.0.join("run1"), d.0.join("run2")]).unwrap();
    (d, ws)
}

fn only(ws: &Workspace, run: &str) -> Query {
    let sources = ws
        .summary()
        .sources
        .iter()
        .filter(|s| s.path.contains(run))
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 1, "one source for {run}");
    Query {
        sources,
        ..Default::default()
    }
}

#[test]
fn every_query_honours_the_source_selection() {
    let (d, ws) = two_runs();
    let summary = ws.summary();
    assert_eq!(summary.total, 8);
    assert_eq!(summary.sources.len(), 2);
    // Same display name: the old `@source`-name workaround could not tell them apart.
    assert_eq!(summary.sources[0].name, summary.sources[1].name);
    let run1 = only(&ws, "run1");
    let run2 = only(&ws, "run2");

    let page = |q: &Query| {
        let req: PageRequest = serde_json::from_value(json!({ "query": q })).unwrap();
        let p = ws.page(&req).unwrap();
        assert!(p.rows.iter().all(|r| q.sources.is_empty() || q.sources.contains(&r.source)));
        p.total
    };
    assert_eq!(page(&run1), 3);
    assert_eq!(page(&run2), 5);
    assert_eq!(page(&Query::default()), 8);
    // Unknown ids match nothing instead of failing.
    assert_eq!(
        page(&Query {
            sources: vec![4242],
            ..Default::default()
        }),
        0
    );

    let histogram_total = |q: &Query| {
        let req: TimelineRequest = serde_json::from_value(json!({ "query": q, "buckets": 10 })).unwrap();
        ws.timeline(&req)
            .unwrap()
            .histogram
            .iter()
            .flatten()
            .map(|&n| n as usize)
            .sum::<usize>()
    };
    assert_eq!(histogram_total(&run1), 3);
    assert_eq!(histogram_total(&run2), 5);

    let facet = |q: &Query| {
        let req: FacetRequest = serde_json::from_value(json!({ "query": q, "field": "Run" })).unwrap();
        let f = ws.facet(&req).unwrap();
        (f.distinct, f.values.first().map(|v| v.value.clone()))
    };
    assert_eq!(facet(&run1), (1, Some("run1".to_string())));
    assert_eq!(facet(&run2), (1, Some("run2".to_string())));

    let chart_points = |q: &Query| {
        let req: ChartRequest = serde_json::from_value(json!({ "query": q, "chartId": "events-by-level", "buckets": 10 })).unwrap();
        let c = ws.chart(&req).unwrap();
        c.series.iter().flat_map(|s| s.points.iter().filter_map(|p| p.1)).sum::<f64>()
    };
    assert_eq!(chart_points(&run1), 3.0);
    assert_eq!(chart_points(&run2), 5.0);

    let stats = |q: &Query| {
        let req: FieldStatsRequest = serde_json::from_value(json!({ "query": q, "field": "Cpu" })).unwrap();
        ws.field_stats(&req).unwrap().count
    };
    assert_eq!(stats(&run1), 3);
    assert_eq!(stats(&run2), 5);

    let ql = ws
        .run_query(&QueryRequest {
            query: Query {
                ql: "| stats count()".into(),
                ..run2.clone()
            },
        })
        .unwrap();
    match ql {
        imt_core::ql::QlResult::Table { rows, .. } => assert_eq!(serde_json::to_value(&rows).unwrap()[0][0], json!(5)),
        other => panic!("expected a table, got {other:?}"),
    }

    let out = d.0.join("run1.csv");
    let export: ExportRequest = serde_json::from_value(json!({ "query": run1, "format": "csv", "path": out.to_string_lossy() })).unwrap();
    assert_eq!(ws.export(&export).unwrap().events, 3);
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(csv.contains("run1 sample") && !csv.contains("run2 sample"));
}

#[test]
fn an_oversized_source_selection_is_refused_with_a_next_step() {
    let (_d, ws) = two_runs();
    let req: PageRequest = serde_json::from_value(json!({
        "query": { "sources": (0..=imt_core::query::MAX_QUERY_SOURCES as u32).collect::<Vec<_>>() }
    }))
    .unwrap();
    let err = ws.page(&req).err().unwrap().to_string();
    assert!(err.contains("clear the source selection"), "{err}");
}
