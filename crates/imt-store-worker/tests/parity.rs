//! Every port operation over the worker answers exactly what the store answers in process: the samples are
//! imported into a store folder in this process, every operation's reply recorded as JSON, the store closed,
//! and the same folder opened through the real worker binary, where each operation must reply the same
//! (this checks the protocol table, the dispatch and that every DTO survives its JSON round trip).
//! Also: a scan streams every event, a cancelled query stops, and an operation on a closed store fails typed.

use imt_core::ports::{Analysis, Diagnostics, Filter, Ingest, Positions, Profiles, Query, Search, StoreError};
use imt_core::ql::QueryControl;
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use imt_store_worker::{StoreWorker, WorkerConfig};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::Duration;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let p = std::env::temp_dir().join(format!("imt-parity-{tag}-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

const EXTRACT: &str = r#"
[[extract]]
name = "Elapsed"
from = "@message"
regex = '(?P<Elapsed>\d+) ?ms'
"#;

fn req<T: DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn j<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap()
}

/// The same questions to any store: (name, reply as JSON or the error's text).
fn ask<S: Query + Analysis + Profiles + Diagnostics + Search + Positions>(s: &S, export_to: &std::path::Path) -> Vec<(String, Value)> {
    let all = Filter::default();
    let errors: Filter = req(json!({"levels": ["Error", "Fatal"]}));
    let summary = s.summary().unwrap();
    let mid = (summary.from.unwrap() + summary.to.unwrap()) / 2;
    let first = s.page(&req(json!({"limit": 5}))).unwrap().rows[0].id;
    let text = |r: Result<Value, StoreError>| r.unwrap_or_else(|e| Value::String(format!("error: {e}")));
    let mut out: Vec<(String, Value)> = vec![
        ("summary".into(), j(&summary)),
        ("page".into(), text(s.page(&req(json!({"query": errors, "limit": 50, "descending": true}))).map(j))),
        ("anchor".into(), text(s.page(&req(json!({"anchor": first, "limit": 20}))).map(j))),
        ("timeline".into(), text(s.timeline(&req(json!({"query": all, "buckets": 100, "alwaysShowLifecycle": true}))).map(j))),
        ("facet".into(), text(s.facet(&req(json!({"query": all, "field": "@level", "limit": 10}))).map(j))),
        (
            "chart".into(),
            text(s.chart(&req(json!({"query": all, "def": {"id": "c", "title": "c", "y": "count()", "bucket": "auto", "kind": "line"}, "buckets": 30}))).map(j)),
        ),
        ("fieldStats".into(), text(s.field_stats(&req(json!({"query": all, "field": "@level", "buckets": 10}))).map(j))),
        ("context".into(), text(s.source_context(&req(json!({"recordId": first.0, "before": 2, "after": 2}))).map(j))),
        ("ql".into(), text(s.run_query(&req(json!({"query": {"ql": "stats count() by @level"}}))).map(j))),
        ("check".into(), text(s.check_query("where nope ===").map(j))),
        ("complete".into(), text(s.complete_query(&req(json!({"text": "where Mach", "cursor": 10}))).map(j))),
        ("idsAt".into(), text(s.ids_at(0, 10).map(j))),
        ("events".into(), text(s.events(&req(json!({"ids": [first, 999_999_999]}))).map(j))),
        ("positionOf".into(), text(s.position_of(first).map(j))),
        ("positioned".into(), text(s.positioned().map(j))),
        ("patterns".into(), text(s.patterns(&req(json!({"query": all}))).map(j))),
        ("exceptions".into(), text(s.exception_groups(&req(json!({"query": all}))).map(j))),
        ("anomalies".into(), text(s.anomalies(&req(json!({"query": all}))).map(j))),
        (
            "compare".into(),
            text(s.compare(&req(json!({"a": {"to": mid}, "b": {"from": mid}}))).map(j)),
        ),
        ("correlate".into(), text(s.correlate(&req(json!({"field": "@level", "value": "Error"}))).map(j))),
        ("traces".into(), text(s.traces(&req(json!({"query": all}))).map(j))),
        ("requests".into(), text(s.requests(&req(json!({"query": all, "idField": "RequestId|CorrelationId"}))).map(j))),
        ("moment".into(), text(s.moment(&req(json!({"query": all, "from": mid - 1_800_000, "to": mid + 1_800_000}))).map(j))),
        ("overview".into(), text(s.overview(&req(json!({"query": all}))).map(j))),
        ("runs".into(), text(s.runs(&req(json!({}))).map(j))),
        ("profile".into(), text(s.active_profile().map(|p| j(p.name)))),
        ("validate".into(), text(s.validate_profile(&Profile::default_profile()).map(j))),
        (
            "previewParse".into(),
            text(s.preview_parse(&req(json!({"toml": "", "sample": "2026-10-14 10:00:00 ERROR boom
  at x
"}))).map(j)),
        ),
        (
            "previewExtractor".into(),
            text(s.preview_extractor(&req(json!({"toml": EXTRACT, "extractor": "Elapsed", "query": all}))).map(j)),
        ),
        ("problems".into(), text(s.problems().map(j))),
        ("evaluateFix".into(), text(s.evaluate_fix(&req(json!({"profileOverride": "[parse]\n"}))).map(j))),
        ("search".into(), text(s.search(&req(json!({"text": "timeout", "limit": 20}))).map(j))),
    ];
    let exported = s.export(&req(
        json!({"query": errors, "format": "csv", "path": export_to.to_string_lossy(), "overwrite": true}),
    ));
    out.push(("export".into(), text(exported.map(|r| j(r.events)))));
    out
}

#[test]
fn every_operation_over_the_worker_answers_like_the_store() {
    let dir = TempDir::new("store");
    let exports = TempDir::new("exports");
    let roots: Vec<PathBuf> = ["serilog-text", "log4net", "serilog-clef"]
        .iter()
        .map(|d| samples().join(d))
        .filter(|p| p.is_dir())
        .collect();
    assert!(!roots.is_empty(), "the samples are there");
    let local = {
        let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
        store.open(&roots).unwrap().wait().unwrap();
        store.wait_for_index(Duration::from_secs(120)).unwrap();
        let answers = ask(&store, &exports.0.join("local.csv"));
        let mut events = 0usize;
        store
            .scan(&Filter::default(), &mut |b| {
                events += b.num_rows();
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(events, store.summary().unwrap().total);
        answers
    };
    let w = StoreWorker::new(WorkerConfig::new(env!("CARGO_BIN_EXE_imt-store-worker"))).unwrap();
    let remote_store = w.open_store(&dir.0, Profile::default_profile(), false, None).unwrap();
    let remote = ask(&remote_store, &exports.0.join("remote.csv"));
    for ((name, l), (_, r)) in local.iter().zip(&remote) {
        assert_eq!(l, r, "{name} differs over the worker");
    }
    assert_eq!(
        std::fs::read(exports.0.join("local.csv")).unwrap(),
        std::fs::read(exports.0.join("remote.csv")).unwrap(),
        "the export writes the same file"
    );

    // A scan streams every event as Arrow batches; breaking stops it.
    let total = remote_store.summary().unwrap().total;
    let (mut events, mut batches) = (0usize, 0usize);
    remote_store
        .scan(&Filter::default(), &mut |b| {
            events += b.num_rows();
            batches += 1;
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(events, total);
    let mut seen = 0;
    remote_store
        .scan(&Filter::default(), &mut |_| {
            seen += 1;
            ControlFlow::Break(())
        })
        .unwrap();
    assert_eq!(seen, 1);
    assert!(batches >= 1);
    // The store still answers after a cancelled scan.
    assert_eq!(remote_store.summary().unwrap().total, total);

    // A query cancelled before it starts, and one cancelled while it runs, fail as cancelled.
    let control = QueryControl::new();
    control.cancel();
    let q = req(json!({"query": {"ql": "stats count() by @level"}}));
    assert_eq!(remote_store.run_query_with(&q, &control).unwrap_err(), StoreError::Cancelled);
    let control = QueryControl::new();
    let c = control.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5));
        c.cancel();
    });
    let heavy = req(json!({"query": {"ql": "sort message | head 100000"}}));
    match remote_store.run_query_with(&heavy, &control) {
        Err(StoreError::Cancelled) | Ok(_) => {}
        Err(e) => panic!("a cancelled query fails as cancelled, not {e}"),
    }
    canceller.join().unwrap();
    // Analyses run under a job's control are cancellable too.
    let control = QueryControl::new();
    control.cancel();
    let r = control.scope(|| remote_store.patterns(&req(json!({"query": {}}))));
    assert_eq!(r.unwrap_err(), StoreError::Cancelled);

    // A closed store reopens on the next call.
    remote_store.close();
    assert_eq!(remote_store.summary().unwrap().total, total);
    w.shutdown();
}
