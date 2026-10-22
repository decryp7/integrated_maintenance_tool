//! The in-memory workspace behind the store ports (`WorkspaceStore`): every port answers what the workspace itself
//! answers (the ports are a thin adapter), mutations go through to the workspace, and the operations it has no
//! equivalent for are `Unsupported`, never a panic.

use imt_core::api::{
    ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExtractPreviewRequest, FacetRequest,
    FieldStatsRequest, MomentRequest, ParsePreviewRequest, QueryRequest, RequestsRequest, TimelineRequest, TraceRequest,
};
use imt_core::batch::{EventId, SourceId};
use imt_core::ports::{
    Analysis, Diagnostics, Ingest, ParserDef, Positions, Profiles, Projects, Query as QueryPort, QuickFix, Search, StoreError,
};
use imt_core::profile_change::ChangeClass;
use imt_core::query::Query;
use imt_core::workspace::WorkspaceStore;
use imt_core::{LogStore, Profile, Workspace};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn req<T: DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn same<T: serde::Serialize, U: serde::Serialize>(a: T, b: U) {
    assert_eq!(serde_json::to_value(a).unwrap(), serde_json::to_value(b).unwrap());
}

fn opened(paths: &[PathBuf]) -> WorkspaceStore {
    let store = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
    store.open(paths).unwrap().wait().unwrap();
    store
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("imt-ports-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn queries_through_the_ports_answer_what_the_workspace_answers() {
    let store = opened(&[samples().join("serilog-clef")]);
    let ws = store.read();
    let errors = Query {
        ql: "level >= warning".into(),
        ..Query::default()
    };

    let t: TimelineRequest = req(json!({"query": errors, "buckets": 50}));
    same(store.timeline(&t).unwrap(), ws.timeline(&t).unwrap());
    let f: FacetRequest = req(json!({"field": "@level"}));
    same(store.facet(&f).unwrap(), ws.facet(&f).unwrap());
    let c: ChartRequest =
        req(json!({"def": {"id": "c", "title": "c", "y": "count()", "bucket": "auto", "split_by": [], "kind": "line", "unit": ""}}));
    same(store.chart(&c).unwrap(), ws.chart(&c).unwrap());
    let s: FieldStatsRequest = req(json!({"field": "ProcessId"}));
    same(store.field_stats(&s).unwrap(), ws.field_stats(&s).unwrap());
    let x: ContextRequest = req(json!({"recordId": 3, "before": 2, "after": 2}));
    same(store.source_context(&x).unwrap(), ws.source_context(&x).unwrap());
    assert_eq!(
        store.check_query("where ===").unwrap().is_some(),
        ws.check_query("where ===").is_some()
    );
    assert!(store.check_query("level >= error").unwrap().is_none());
    let comp = CompletionRequest {
        text: "lev".into(),
        cursor: 3,
    };
    same(store.complete_query(&comp).unwrap(), ws.complete_query(&comp));

    let q = QueryRequest { query: errors.clone() };
    same(store.patterns(&q).unwrap().result, ws.patterns(&q).unwrap());
    same(store.exception_groups(&q).unwrap().result, ws.exception_groups(&q).unwrap());
    same(store.anomalies(&q).unwrap().result, ws.anomalies(&q).unwrap());
    same(store.overview(&q).unwrap().result, ws.overview(&q).unwrap());
    let analysed = store.patterns(&q).unwrap().coverage;
    assert!(analysed.analysed > 0 && analysed.analysed <= analysed.matched, "{analysed:?}");

    let summary = QueryPort::summary(&store).unwrap();
    let (from, to) = (summary.from.unwrap(), summary.to.unwrap());
    let cmp = CompareRequest {
        a: Query {
            from: Some(from),
            to: Some((from + to) / 2),
            ..Query::default()
        },
        b: Query {
            from: Some((from + to) / 2 + 1),
            to: Some(to),
            ..Query::default()
        },
    };
    let compared = store.compare(&cmp).unwrap();
    same(&compared.result, ws.compare(&cmp).unwrap());
    assert!(compared.coverage.matched > 0);
    let m: MomentRequest = req(json!({"from": from, "to": from + 60_000}));
    same(store.moment(&m).unwrap().result, ws.moment(&m).unwrap());
    let tr: TraceRequest = req(json!({}));
    same(store.traces(&tr).unwrap().result, ws.traces(&tr).unwrap());
    let rq: RequestsRequest = req(json!({}));
    same(store.requests(&rq).unwrap().result, ws.requests(&rq).unwrap());
    let corr = CorrelateRequest {
        field: "MachineName".into(),
        value: "WEB01".into(),
    };
    let correlated = store.correlate(&corr).unwrap();
    assert_eq!(correlated.result.len() as u64, correlated.coverage.matched);
    assert_eq!(
        correlated.result,
        ws.correlate(&corr).into_iter().map(|i| EventId(u64::from(i))).collect::<Vec<_>>()
    );
    same(store.runs(&Default::default()).unwrap(), ws.runs(&Default::default()));

    // Bad input is a typed error, not a panic.
    let bad: FacetRequest = req(json!({"field": "@level", "query": {"ql": "where ==="}}));
    assert!(matches!(store.facet(&bad), Err(StoreError::Invalid(_))), "{:?}", store.facet(&bad));
    let index = store.index_status().unwrap();
    assert_eq!((index.total, index.ready), (summary.total as u64, false));
}

#[test]
fn profile_ports_validate_preview_and_swap_the_profile() {
    let store = opened(&[samples().join("serilog-clef")]);
    assert_eq!(store.active_profile().unwrap().name, Profile::default_profile().name);
    store.validate_profile(&Profile::default_profile()).unwrap();
    // A profile that parses but does not compile (an invalid regex) is refused with a message.
    let broken = Profile::from_toml(
        "[[extract]]
name = \"x\"
from = \"@message\"
regex = \"(\"
",
    )
    .unwrap();
    assert!(matches!(store.validate_profile(&broken), Err(StoreError::Invalid(_))));

    let p: ParsePreviewRequest = req(json!({"toml": "", "sample": "2026-10-14 10:00:00 INFO hello\n", "fileName": "a.log"}));
    same(store.preview_parse(&p).unwrap(), LogStore::preview_parse(&*store.read(), &p));
    let e: ExtractPreviewRequest = req(json!({"toml": "", "extractor": "x"}));
    same(
        store.preview_extractor(&e).unwrap(),
        LogStore::preview_extractor(&*store.read(), &e),
    );

    let before = QueryPort::summary(&store).unwrap().total;
    let mut renamed = Profile::default_profile();
    renamed.name = "Renamed".into();
    store.set_profile(renamed).unwrap().wait().unwrap();
    assert_eq!(store.active_profile().unwrap().name, "Renamed");
    assert_eq!(QueryPort::summary(&store).unwrap().total, before);
}

#[test]
fn parser_ports_report_assign_and_classify() {
    let store = opened(&[samples().join("serilog-clef"), samples().join("csv")]);
    let uses = store.parsers().unwrap();
    assert!(!uses.is_empty());
    let sources: usize = uses.iter().map(|u| u.sources.len()).sum();
    assert_eq!(sources, store.read().sources().len(), "every source is read by one parser");
    assert!(uses
        .windows(2)
        .all(|w| (&w[0].parser.id, &w[0].parser.hash) <= (&w[1].parser.id, &w[1].parser.hash)));
    for u in &uses {
        assert!(u.sources.windows(2).all(|w| w[0].0 < w[1].0));
    }

    // A changed version of a parser in use touches its sources; an unrelated one touches none.
    let used = &uses[0];
    let lib = imt_core::parser::library::Library::builtin_only();
    let current = lib.get(&used.parser.id).expect("a built-in parser");
    let changed = current.parser.with_override("description = \"changed for a test\"").unwrap();
    let change = store
        .classify_parser(&ParserDef {
            id: used.parser.id.clone(),
            version: changed.version.to_string(),
            toml: changed.canonical_toml(),
        })
        .unwrap();
    assert_eq!(change.class, ChangeClass::Parsing);
    assert_eq!(change.sources, used.sources);
    assert_eq!(change.sections, ["format"]);
    let same_parser = store
        .classify_parser(&ParserDef {
            id: used.parser.id.clone(),
            version: current.parser.version.to_string(),
            toml: current.parser.canonical_toml(),
        })
        .unwrap();
    assert_eq!((same_parser.class, same_parser.sources.len()), (ChangeClass::Presentation, 0));
    assert!(matches!(
        store.classify_parser(&ParserDef {
            id: "x".into(),
            version: "1".into(),
            toml: "nope".into()
        }),
        Err(StoreError::Invalid(_))
    ));

    // Assigning: unknown sources and parsers are NotFound; a known one re-reads that source with it (the source
    // gets a new id), and clearing the override goes back to the automatic choice.
    let count = |id: &str| {
        store
            .parsers()
            .unwrap()
            .iter()
            .filter(|u| u.parser.id == id)
            .map(|u| u.sources.len())
            .sum::<usize>()
    };
    let clef = store
        .parsers()
        .unwrap()
        .into_iter()
        .find(|u| u.parser.id == "serilog-clef")
        .expect("the clef samples");
    let (clefs, generics) = (count("serilog-clef"), count("generic"));
    assert!(matches!(store.assign_parser(SourceId(9_999), None), Err(StoreError::NotFound(_))));
    assert!(matches!(
        store.assign_parser(clef.sources[0], Some("no-such-parser")),
        Err(StoreError::NotFound(_))
    ));
    store.assign_parser(clef.sources[0], Some("generic")).unwrap().wait().unwrap();
    assert_eq!((count("serilog-clef"), count("generic")), (clefs - 1, generics + 1));
    let moved = store
        .parsers()
        .unwrap()
        .into_iter()
        .find(|u| u.parser.id == "generic")
        .unwrap()
        .sources
        .into_iter()
        .max_by_key(|s| s.0)
        .unwrap();
    store.assign_parser(moved, None).unwrap().wait().unwrap();
    assert_eq!(
        (count("serilog-clef"), count("generic")),
        (clefs, generics),
        "back to the automatic choice"
    );

    let setup = {
        let ws = store.read();
        ws.parsers().setup().clone()
    };
    assert!(store.set_parsers(&setup).unwrap().is_empty(), "the same setup changes no source");
    store.reread_parsers().unwrap().wait().unwrap();
}

#[test]
fn ingest_ports_reload_refresh_tail_restore_and_remove() {
    let dir = TempDir::new("ingest");
    let log = dir.0.join("app.log");
    std::fs::write(&log, "2026-10-14 10:00:00 INFO one\n2026-10-14 10:00:01 INFO two\n").unwrap();
    let store = opened(std::slice::from_ref(&dir.0));
    assert_eq!(QueryPort::summary(&store).unwrap().total, 2);

    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"2026-10-14 10:00:02 INFO three\n")
        .unwrap();
    assert_eq!(store.append_tail(std::slice::from_ref(&log)).unwrap().appended, 1);
    assert_eq!(QueryPort::summary(&store).unwrap().total, 3);

    // Rewritten: what was read stays (detached) and the new content is read from the start.
    std::fs::write(&log, "2026-10-14 10:00:00 INFO rewritten\n").unwrap();
    store.refresh(std::slice::from_ref(&log)).unwrap().wait().unwrap();
    assert_eq!(QueryPort::summary(&store).unwrap().total, 3 + 1);
    std::fs::write(&log, "2026-10-14 10:00:00 INFO a\n2026-10-14 10:00:00 INFO b\n").unwrap();
    let report = store.reload().unwrap().wait().unwrap();
    assert_eq!(report.events, 2);

    let state = store.read().state();
    let saved = store.save_project(&Default::default()).unwrap();
    assert_eq!(saved.roots, state.roots);
    store.clear().unwrap();
    assert_eq!(QueryPort::summary(&store).unwrap().total, 0);
    store.restore(&state).unwrap().wait().unwrap();
    assert_eq!(QueryPort::summary(&store).unwrap().total, 2, "restored from the saved state");

    let id = store.read().sources()[0].id;
    store.remove_source(SourceId(id)).unwrap();
    assert_eq!(QueryPort::summary(&store).unwrap().total, 0);
    assert!(matches!(store.remove_source(SourceId(id)), Err(StoreError::NotFound(_))));

    // An open that fails is an error of the session, not a panic.
    let missing = store.open(&[dir.0.join("missing.log")]);
    if let Ok(session) = missing {
        let _ = session.wait();
    }
}

#[test]
fn exports_go_through_the_port() {
    let dir = TempDir::new("export");
    let store = opened(&[samples().join("serilog-clef")]);
    let out = dir.0.join("out.clef");
    let r: ExportRequest = req(json!({"format": "clef", "path": out.to_string_lossy(), "query": {"ql": "level >= error"}}));
    let result = store.export(&r).unwrap();
    let lines = std::fs::read_to_string(&out).unwrap().lines().count();
    assert!(lines > 0);
    assert_eq!(
        serde_json::to_value(&result).unwrap()["count"].as_u64().unwrap_or(lines as u64),
        lines as u64
    );
    // Refused without overwrite.
    assert!(matches!(store.export(&r), Err(StoreError::Invalid(_))));
    let _: &Path = &out;
}

#[test]
fn what_the_workspace_cannot_do_is_unsupported() {
    let store = opened(&[samples().join("serilog-clef")]);
    let unsupported = |r: Result<(), StoreError>| assert!(matches!(r, Err(StoreError::Unsupported { .. })), "{r:?}");
    unsupported(store.ids_at(0, 1).map(|_| ()));
    unsupported(store.position_of(EventId(0)).map(|_| ()));
    unsupported(store.positioned().map(|_| ()));
    unsupported(store.scan(&Default::default(), &mut |_| std::ops::ControlFlow::Continue(())));
    unsupported(store.propose_span_rule(&req(json!({"field": "x"}))).map(|_| ()));
    unsupported(store.export_project(&Default::default(), Path::new("x.imtz")).map(|_| ()));
    let fix: Result<QuickFix, _> = serde_json::from_value(json!({"kind": "none"}));
    if let Ok(fix) = fix {
        unsupported(store.evaluate_fix(&fix).map(|_| ()));
    }
    assert!(store.problems().unwrap().is_empty());
    assert_eq!(store.store_dir(), PathBuf::new());
    // A poisoned lock is recovered, not propagated.
    let s = std::sync::Arc::new(opened(&[samples().join("serilog-clef")]));
    let t = s.clone();
    let _ = std::thread::spawn(move || {
        let _guard = t.write();
        panic!("poison the lock");
    })
    .join();
    assert!(QueryPort::summary(&*s).unwrap().total > 0);
}
