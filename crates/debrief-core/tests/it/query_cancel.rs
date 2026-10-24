//! Cancelling long query-language runs (`LogStore::run_query_with`): a cancelled run stops promptly,
//! returns `RunError::Cancelled`, caches nothing partial and leaves the workspace usable.

use debrief_core::api::QueryRequest;
use debrief_core::ql::{QlResult, QueryControl, QueryStage, RunError};
use debrief_core::query::Query;
use debrief_core::{LogStore, Profile, Workspace, DEFAULT_PROFILE_TOML};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const RECORDS: usize = 120_000;

/// One CLEF file of `RECORDS` events: every 10th is a warning, `Took` cycles through 0..1000.
fn store() -> &'static (dyn LogStore + 'static) {
    static STORE: OnceLock<Box<dyn LogStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!("debrief-cancel-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&dir).unwrap();
            let mut text = String::with_capacity(RECORDS * 120);
            for i in 0..RECORDS {
                let level = if i % 10 == 0 { "Warning" } else { "Information" };
                let (m, s) = ((i / 60) % 60, i % 60);
                text.push_str(&format!(
                    "{{\"@t\":\"2026-10-14T10:{m:02}:{s:02}Z\",\"@l\":\"{level}\",\"@mt\":\"request {i} done\",\"Took\":{},\"Host\":\"h{}\"}}\n",
                    i % 1000,
                    i % 7
                ));
            }
            let file: PathBuf = dir.join("big.clef");
            std::fs::write(&file, text).unwrap();
            let mut ws: Box<dyn LogStore> = Box::new(Workspace::new(Profile::from_toml(DEFAULT_PROFILE_TOML).unwrap()).unwrap());
            ws.open(&[file]).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            ws
        })
        .as_ref()
}

fn req(ql: &str, search: &str) -> QueryRequest {
    QueryRequest {
        query: Query {
            ql: ql.into(),
            text: search.into(),
            ..Default::default()
        },
    }
}

fn warnings(out: QlResult) -> usize {
    match out {
        QlResult::Events { ids, .. } => ids.len(),
        QlResult::Table { .. } => panic!("expected events"),
    }
}

#[test]
fn a_cancelled_control_stops_before_any_work_and_caches_nothing() {
    let s = store();
    let control = QueryControl::new();
    control.cancel();
    let r = req("level = warning", "request");
    assert_eq!(s.run_query_with(&r, &control), Err(RunError::Cancelled));
    // The same query afterwards (same matching cache key) is complete.
    let out = s.run_query_with(&r, &QueryControl::new()).unwrap();
    assert_eq!(warnings(out), RECORDS / 10);
}

#[test]
fn cancelling_a_running_query_stops_it_promptly_and_leaves_the_workspace_usable() {
    let s = store();
    // A full-text search no other test uses, so matching really runs, then sort + stats.
    let r = req("Took >= 0 | sort Took desc | stats count() as n, p95(Took) as p by Host", "done");
    let control = QueryControl::new();
    let watcher = control.clone();
    let cancelled_at = std::thread::scope(|scope| {
        let run = scope.spawn(|| s.run_query_with(&r, &control));
        // Cancel as soon as the run reports progress.
        let start = Instant::now();
        while watcher.progress().done == 0 && start.elapsed() < Duration::from_secs(30) {
            std::thread::yield_now();
        }
        let progress = watcher.progress();
        watcher.cancel();
        let at = Instant::now();
        let out = run.join().unwrap();
        assert_eq!(out, Err(RunError::Cancelled), "progress when cancelled: {progress:?}");
        at.elapsed()
    });
    assert!(cancelled_at < Duration::from_secs(2), "took {cancelled_at:?} to stop");

    // Usable afterwards: the same query, uncancelled, is complete and reports full progress.
    let control = QueryControl::new();
    let out = s.run_query_with(&r, &control).unwrap();
    let QlResult::Table { columns, rows } = out else {
        panic!("expected a table")
    };
    assert_eq!(columns, ["Host", "n", "p"]);
    let total: u64 = rows.iter().map(|r| r[1].as_u64().unwrap()).sum();
    assert_eq!(total as usize, RECORDS);
    let p = control.progress();
    assert_eq!((p.stage, p.done, p.total), (QueryStage::Evaluating, RECORDS as u64, RECORDS as u64));
    // And the uncancellable entry point agrees.
    assert_eq!(warnings(s.run_query(&req("level = warning", "done")).unwrap()), RECORDS / 10);
}

#[test]
fn syntax_errors_stay_errors() {
    let s = store();
    match s.run_query_with(&req("level >", ""), &QueryControl::new()) {
        Err(RunError::Invalid(e)) => assert!(e.offset > 0),
        other => panic!("expected a syntax error, got {other:?}"),
    }
    // A workspace filter that does not compile (bad regex) is an error at no particular span.
    let mut bad = req("level = warning", "(");
    bad.query.regex = true;
    match s.run_query_with(&bad, &QueryControl::new()) {
        Err(RunError::Invalid(e)) => assert_eq!((e.offset, e.len), (0, 0)),
        other => panic!("expected an error, got {other:?}"),
    }
}
