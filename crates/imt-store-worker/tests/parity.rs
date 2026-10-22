//! Every port operation over the worker answers exactly what the store answers in process: the samples are
//! imported into a store folder in this process, every operation's reply recorded as JSON, the store closed,
//! and the same folder opened through the real worker binary, where each operation must reply the same
//! (this checks the protocol table, the dispatch and that every DTO survives its JSON round trip).
//! Also: a scan streams every event, a cancelled query stops, and an operation on a closed store fails typed.

use imt_core::ports::{Analysis, Diagnostics, Filter, Ingest, Positions, Profiles, Projects, Query, Search, StoreError};
use imt_core::ql::QueryControl;
use imt_core::Profile;
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
        ("parsers".into(), text(s.parsers().map(j))),
        (
            "previewSpans".into(),
            text(s.preview_spans(&req(json!({"draft": {"kind": "work", "label": "Work", "start": {"type": "pattern", "pattern": "started"}, "stop": {"type": "pattern", "pattern": "stopped"}}}))).map(j)),
        ),
        ("evaluateFix".into(), text(s.evaluate_fix(&req(json!({"parserOverride": "[format]\n"}))).map(j))),
        ("setParsers".into(), text(s.set_parsers(&imt_core::parser::ParserSetup::builtins()).map(j))),
        ("search".into(), text(s.search(&req(json!({"text": "timeout", "limit": 20}))).map(j))),
        ("wordPage".into(), text(s.page(&req(json!({"query": {"text": "time", "textMode": "wordsOrSubstring"}, "limit": 20}))).map(j))),
        ("partialMatches".into(), text(s.partial_matches(&req(json!({"text": "imeout", "textMode": "words"}))).map(j))),
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
        // The same importers as the worker: the plugin processes next to the test binaries.
        let plugins = imt_plugin_host::Builtins::load(&imt_plugin_host::builtins::default_binary()).plugins();
        let registry = imt_store_worker::server::install_plugins(plugins, &[]);
        let store = DuckStore::create(&dir.0, Profile::default_profile(), registry).unwrap();
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

/// What a store read from the mixed samples, independent of the ids it gave (sources by path, events by line).
fn mixed_answers<S: Query>(s: &S) -> Vec<(String, Value)> {
    let summary = s.summary().unwrap();
    let path_of: std::collections::HashMap<u32, String> = summary.sources.iter().map(|x| (x.id, x.path.clone())).collect();
    let mut sources: Vec<Value> = summary
        .sources
        .iter()
        .map(|x| json!([x.path, x.importer, x.records, x.skipped, x.parser, j(&x.problems), x.from, x.to]))
        .collect();
    sources.sort_by_key(|v| v.to_string());
    let mut out = vec![
        ("total".into(), json!([summary.total, summary.untimed, summary.from, summary.to])),
        ("instance key".into(), j(&summary.default_group_by)),
        (
            "fields".into(),
            j(summary.fields.iter().map(|f| (&f.name, f.count, f.distinct)).collect::<Vec<_>>()),
        ),
        ("sources".into(), Value::Array(sources)),
    ];
    let timeline = s
        .timeline(&req(json!({"query": {}, "buckets": 200, "alwaysShowLifecycle": true})))
        .unwrap();
    let lanes: Vec<Value> = timeline.lanes.iter().map(|l| json!([l.key, l.total, l.first, l.last])).collect();
    out.push(("lanes".into(), json!([timeline.histogram, lanes])));
    for field in ["@level", "Application", "Logger", "Host", "App", "ProcId", "ProcessId"] {
        let f = s.facet(&req(json!({"query": {}, "field": field, "limit": 50}))).unwrap();
        let values: Vec<Value> = f.values.iter().map(|v| json!([v.value, v.count])).collect();
        out.push((format!("facet {field}"), json!([f.total, f.distinct, values])));
    }
    let page = s.page(&req(json!({"query": {}, "offset": 0, "limit": 5000}))).unwrap();
    let mut rows: Vec<Value> = page
        .rows
        .iter()
        .map(|r| json!([path_of[&r.source.0], r.line, r.ts, j(r.level), r.message, r.group, j(&r.fields)]))
        .collect();
    rows.sort_by_key(|v| v.to_string());
    out.push(("rows".into(), json!([page.total, rows])));
    out
}

/// Mixed sources (ADR 0020) imported by the worker read exactly as imported in process: one file of several
/// products read line by line with their own formats (and lines no format reads), a log server's syslog from
/// several hosts and apps (the syslog plugin, run by the worker), and the instance key chosen per source.
#[test]
fn mixed_sources_imported_by_the_worker_read_like_in_process() {
    let paths: Vec<PathBuf> = ["linux/logserver01/apps-20261014.log", "linux/logserver01/syslog-20261014.log"]
        .iter()
        .map(|p| samples().join(p))
        .collect();
    let local_dir = TempDir::new("mixed-local");
    let plugins = imt_plugin_host::Builtins::load(&imt_plugin_host::builtins::default_binary()).plugins();
    let registry = imt_store_worker::server::install_plugins(plugins, &[]);
    let local = DuckStore::create(&local_dir.0, Profile::default_profile(), registry).unwrap();
    local.open(&paths).unwrap().wait().unwrap();
    let local_answers = mixed_answers(&local);
    drop(local);

    let remote_dir = TempDir::new("mixed-remote");
    let w = StoreWorker::new(WorkerConfig::new(env!("CARGO_BIN_EXE_imt-store-worker"))).unwrap();
    let remote = w.open_store(&remote_dir.0, Profile::default_profile(), true, None).unwrap();
    remote.open(&paths).unwrap().wait().unwrap();
    let remote_answers = mixed_answers(&remote);
    for ((name, l), (_, r)) in local_answers.iter().zip(&remote_answers) {
        assert_eq!(l, r, "{name} differs between the worker's import and the import in process");
    }
    let summary = remote.summary().unwrap();
    assert!(
        summary
            .sources
            .iter()
            .any(|s| s.problems.iter().any(|p| p.kind == imt_core::ingest::ProblemKind::UnmatchedLine)),
        "the lines no format reads are reported"
    );
    assert!(
        summary
            .sources
            .iter()
            .any(|s| s.path.ends_with("syslog-20261014.log") && s.records > 0),
        "{summary:?}"
    );
    w.shutdown();
}

/// The redaction summary over the worker equals the store's own, and is cancellable.
#[test]
fn the_redaction_summary_over_the_worker_answers_like_the_store() {
    use imt_core::ports::Redaction;
    let dir = TempDir::new("redact");
    let profile = Profile::from_toml("name = 'Redacting'\n[redact]\ndetectors = ['email', 'ipv4']\n").unwrap();
    let plugins = imt_plugin_host::Builtins::load(&imt_plugin_host::builtins::default_binary()).plugins();
    let registry = imt_store_worker::server::install_plugins(plugins, &[]);
    let local = DuckStore::create(&dir.0, profile.clone(), registry).unwrap();
    local.open(&[samples().join("serilog-clef")]).unwrap().wait().unwrap();
    let request: imt_core::redact::RedactionSummaryRequest = req(json!({"query": {}, "samples": 4}));
    let expected = local.redaction_summary(&request).unwrap();
    assert!(expected.changed > 0, "the samples hold something to redact");
    drop(local);

    let w = StoreWorker::new(WorkerConfig::new(env!("CARGO_BIN_EXE_imt-store-worker"))).unwrap();
    let remote = w.open_store(&dir.0, profile, false, None).unwrap();
    assert_eq!(j(remote.redaction_summary(&request).unwrap()), j(&expected));
    let control = QueryControl::new();
    control.cancel();
    assert_eq!(
        control.scope(|| remote.redaction_summary(&request)).unwrap_err(),
        StoreError::Cancelled
    );
    w.shutdown();
}

/// The same live tail and clock-skew session on any store, answers as JSON with file names for paths.
fn live_session<S: imt_core::ports::Store>(s: &S, logs: &std::path::Path) -> Vec<(String, Value)> {
    let name = |v: Value| -> Value { serde_json::from_str(&v.to_string().replace(&logs.to_string_lossy().into_owned(), "")).unwrap() };
    let clef = |host: &str, n: i64, skew: i64| {
        format!("{{\"@t\":\"{}\",\"@m\":\"{host} {n}\",\"CorrelationId\":\"c{n}\"}}\n", {
            let ms = 1_791_972_000_000 + n * 7_000 + skew;
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string()
        })
    };
    let mut out = Vec::new();
    s.open(&[logs.to_path_buf()]).unwrap().wait().unwrap();
    out.push(("tailFiles".to_string(), name(j(s.tail_files().unwrap()))));
    let suggest = s.suggest_offsets(&req(json!({}))).unwrap();
    out.push((
        "suggest".into(),
        j(suggest
            .suggestions
            .iter()
            .map(|x| (x.suggested_ms, x.pairs, x.agreeing))
            .collect::<Vec<_>>()),
    ));
    let web02 = s.summary().unwrap().sources.iter().find(|x| x.path.contains("web02")).unwrap().id;
    let change = s
        .set_source_offset(&req(json!({"kind": "manual", "source": web02, "offsetMs": 2500, "track": []})))
        .unwrap();
    out.push(("change".into(), j(change)));
    out.push(("offsets".into(), name(j(s.clock_offsets().unwrap()))));
    let mut f = std::fs::OpenOptions::new().append(true).open(logs.join("web02/app.clef")).unwrap();
    std::io::Write::write_all(&mut f, clef("web02", 9, -2_500).as_bytes()).unwrap();
    drop(f);
    let r = s.append_tail(&[logs.join("web02/app.clef")]).unwrap();
    out.push((
        "appendTail".into(),
        j((
            r.appended,
            r.held.len(),
            r.added.len(),
            r.truncated.len(),
            r.reimported.len(),
            r.deleted.len(),
        )),
    ));
    let page = s.page(&req(json!({"query": {}, "limit": 100}))).unwrap();
    out.push((
        "times".into(),
        j(page.rows.iter().map(|r| (r.message.clone(), r.ts)).collect::<Vec<_>>()),
    ));
    out.push((
        "tailFilesAfter".into(),
        name(j(s
            .tail_files()
            .unwrap()
            .iter()
            .map(|f| (f.name.clone(), f.offset, f.lines, f.read))
            .collect::<Vec<_>>())),
    ));
    out
}

/// Clock skew and live tail over the worker answer exactly what the store answers in process.
#[test]
fn clock_skew_and_live_tail_over_the_worker_answer_like_the_store() {
    let make_logs = |tag: &str| {
        let logs = TempDir::new(tag);
        for (host, skew) in [("web01", 0i64), ("web02", -2_500)] {
            std::fs::create_dir_all(logs.0.join(host)).unwrap();
            let text: String = (0..8i64)
                .map(|n| {
                    let ms = 1_791_972_000_000 + n * 7_000 + skew;
                    let t = chrono::DateTime::from_timestamp_millis(ms)
                        .unwrap()
                        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                        .to_string();
                    format!("{{\"@t\":\"{t}\",\"@m\":\"{host} {n}\",\"CorrelationId\":\"c{n}\"}}\n")
                })
                .collect();
            std::fs::write(logs.0.join(host).join("app.clef"), text).unwrap();
        }
        logs
    };
    let (l1, l2) = (make_logs("skew-local"), make_logs("skew-remote"));
    let (d1, d2) = (TempDir::new("skew-store-local"), TempDir::new("skew-store-remote"));
    let plugins = imt_plugin_host::Builtins::load(&imt_plugin_host::builtins::default_binary()).plugins();
    let registry = imt_store_worker::server::install_plugins(plugins, &[]);
    let local = DuckStore::create(&d1.0, Profile::default_profile(), registry).unwrap();
    let expected = live_session(&local, &std::fs::canonicalize(&l1.0).unwrap());
    let w = StoreWorker::new(WorkerConfig::new(env!("CARGO_BIN_EXE_imt-store-worker"))).unwrap();
    let remote = w.open_store(&d2.0, Profile::default_profile(), true, None).unwrap();
    let got = live_session(&remote, &std::fs::canonicalize(&l2.0).unwrap());
    for ((name, e), (_, g)) in expected.iter().zip(&got) {
        assert_eq!(e, g, "{name} differs over the worker");
    }
    assert!(
        expected
            .iter()
            .any(|(n, v)| n == "suggest" && v.as_array().is_some_and(|a| !a.is_empty())),
        "{expected:?}"
    );

    // Opening at the end goes through the worker too; a cancelled suggestion fails as cancelled.
    let at_end = TempDir::new("skew-at-end");
    std::fs::write(at_end.0.join("x.clef"), "{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"old\"}\n").unwrap();
    let report = imt_core::ports::Tail::open_at_end(&remote, std::slice::from_ref(&at_end.0))
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(report.events, 0);
    let control = QueryControl::new();
    control.cancel();
    assert_eq!(
        control
            .scope(|| imt_core::ports::Skew::suggest_offsets(&remote, &req(json!({}))))
            .unwrap_err(),
        StoreError::Cancelled
    );
    w.shutdown();
}
