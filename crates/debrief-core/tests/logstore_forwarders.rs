//! The in-memory workspace used as a `dyn LogStore`: the operations the shell reaches only through the trait
//! answer what the workspace itself answers, and the progress flag is the one `cancel` sets.

use debrief_core::api::QueryRequest;
use debrief_core::query::Query;
use debrief_core::spans::{RunsRequest, SpanDraft, SpanPreviewRequest};
use debrief_core::workspace::ImportProgress;
use debrief_core::{LogStore, Profile, Workspace};
use std::sync::atomic::Ordering;

fn store(tag: &str) -> (Box<dyn LogStore>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("debrief-forward-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text: String = [
        ("Application started", 0),
        ("work", 1),
        ("Application stopped", 2),
        ("Application started", 3),
        ("work", 4),
    ]
    .iter()
    .map(|(m, i)| format!("2026-10-14 09:00:0{i}.000 +00:00 [INF] {m}\n"))
    .collect();
    std::fs::write(dir.join("app.log"), text).unwrap();
    let mut ws: Box<dyn LogStore> = Box::new(Workspace::new(Profile::default_profile()).unwrap());
    ws.open(std::slice::from_ref(&dir)).unwrap();
    (ws, dir)
}

#[test]
fn coverage_runs_and_span_previews_come_through_the_trait() {
    let (ws, dir) = store("trait");
    let all = QueryRequest { query: Query::default() };
    let coverage = ws.analysis_coverage(&all).unwrap();
    assert_eq!((coverage.total, coverage.analysed), (5, 5));
    let narrowed = QueryRequest {
        query: Query {
            text: "work".into(),
            ..Query::default()
        },
    };
    assert_eq!(ws.analysis_coverage(&narrowed).unwrap().total, 2);

    // The built-in run rule pairs the two starts with the stop between them.
    let runs = ws.runs(&RunsRequest::default());
    assert!(!runs.runs.is_empty(), "{runs:?}");
    let none = ws.runs(&RunsRequest {
        kind: Some("no-such-kind".into()),
        ..RunsRequest::default()
    });
    assert!(none.runs.is_empty());

    let draft: SpanDraft = serde_json::from_value(serde_json::json!({
        "kind": "svc", "label": "Service", "start": {"type": "markers", "kinds": ["start"]}, "stop": {"type": "markers", "kinds": ["stop"]}, "per": "instance"
    }))
    .unwrap();
    let preview = ws.preview_spans(&SpanPreviewRequest { draft }).unwrap();
    assert!(preview.errors.is_empty() && !preview.runs.is_empty(), "{preview:?}");

    // The configuration parsers include the built-in formats.
    assert!(ws
        .config_parsers()
        .formats()
        .iter()
        .any(|f| f.id == "json" || f.id == "yaml" || f.id == "toml"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_cancel_flag_is_the_one_cancel_sets() {
    let progress = ImportProgress::default();
    assert!(!progress.cancel_flag().load(Ordering::SeqCst));
    progress.cancel();
    assert!(progress.cancel_flag().load(Ordering::SeqCst));
    assert!(progress.snapshot().cancelled);
}
