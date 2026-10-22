//! Runs between lifecycle markers (`[[span]]`) end to end: a log shared by
//! several instances, the annotations and Runs table, the guided editor's
//! preview, live tail (incremental equals a full recompute) and /samples.

use imt_core::plugin::annotate::AnnotationsRequest;
use imt_core::spans::{RunSort, RunStatus, RunsRequest, SpanPreviewRequest};
use imt_core::{Profile, Workspace};
use std::path::PathBuf;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-spans-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
    fn append(&self, name: &str, text: &str) -> PathBuf {
        use std::io::Write;
        let p = self.0.join(name);
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(text.as_bytes()).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A Serilog file-sink line at minute `m` from process `pid`.
fn line(m: u32, pid: u32, msg: &str) -> String {
    format!(
        "2026-10-14 10:{:02}:00.000 +00:00 [INF] {msg} {{\"MachineName\": \"WORKER01\", \"ProcessId\": {pid}}}\n",
        m
    )
}

const MIN: i64 = 60_000;

/// Two workers sharing one file: 2208 runs 0→30, 2210 runs 5→20 and starts again at 40 (still running);
/// 2208 starts again at 45 without a stop before.
fn shared_file() -> String {
    [
        line(0, 2208, "Application started"),
        line(5, 2210, "Worker service started"),
        line(10, 2208, "processing batch"),
        line(12, 2210, "processing batch"),
        line(20, 2210, "Worker service stopped"),
        line(30, 2208, "Application stopping"),
        line(40, 2210, "Worker service started"),
        line(42, 2210, "processing batch"),
        line(44, 2208, "late batch"),
        line(45, 2208, "Application started"),
        line(50, 2208, "processing batch"),
    ]
    .concat()
}

fn open(dir: &Dir) -> Workspace {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir.0)).unwrap();
    ws
}

fn json(v: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(v).unwrap()
}

#[test]
fn shared_file_runs_per_instance_lane() {
    let dir = Dir::new("shared");
    dir.write("worker.txt", &shared_file());
    let ws = open(&dir);
    let reply = ws.runs(&RunsRequest::default());
    let got: Vec<(String, RunStatus, i64)> = reply
        .runs
        .iter()
        .map(|a| (a.instance.clone().unwrap(), a.run.as_ref().unwrap().status, (a.to - a.from) / MIN))
        .collect();
    assert_eq!(
        got,
        vec![
            ("2208".into(), RunStatus::Closed, 30),
            ("2210".into(), RunStatus::Closed, 15),
            ("2210".into(), RunStatus::Open, 2),
            ("2208".into(), RunStatus::Open, 5),
        ]
    );
    let s = &reply.summaries[0];
    assert_eq!((s.kind.as_str(), s.runs, s.closed, s.open, s.lanes), ("run", 4, 2, 2, 2));
    assert_eq!(s.max_ms, 30 * MIN);
    let first = &reply.runs[0];
    let info = first.run.as_ref().unwrap();
    assert_eq!(info.start.as_ref().unwrap().message, "Application started");
    assert_eq!(info.stop.as_ref().unwrap().message, "Application stopping");
    assert_eq!(first.owner, "profile");

    // The same runs are annotations, within the requested range and lane.
    let anns = ws.annotations(&AnnotationsRequest {
        from: Some(first.from + 35 * MIN),
        to: None,
        instance: Some("2210".into()),
    });
    assert_eq!(anns.len(), 1);
    assert_eq!(anns[0].run.as_ref().unwrap().status, RunStatus::Open);
    // Clicking a run filters its lane and time range.
    let page = ws
        .page(&imt_core::workspace::PageRequest {
            query: first.evidence_query.clone(),
            offset: 0,
            limit: 50,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert_eq!(page.total, 3, "start, batch and stop of 2208");
    assert_eq!(reply.rules[0].kind, "run");
}

#[test]
fn the_runs_table_sorts_by_status_from_runs_without_a_start_to_closed_runs() {
    let dir = Dir::new("status");
    // 3000: a stop with no start, a start that is followed by another before any stop (a crash), a closed run, an open one.
    let text = [
        line(0, 3000, "Application stopping"),
        line(5, 3000, "Application started"),
        line(6, 3000, "processing batch"),
        line(10, 3000, "Application started"),
        line(15, 3000, "Application stopping"),
        line(20, 3000, "Application started"),
    ]
    .concat();
    dir.write("worker.txt", &text);
    let ws = open(&dir);
    let statuses = |descending: bool| -> Vec<RunStatus> {
        ws.runs(&RunsRequest {
            sort: RunSort::Status,
            descending,
            ..Default::default()
        })
        .runs
        .iter()
        .map(|a| a.run.as_ref().unwrap().status)
        .collect()
    };
    let up = statuses(false);
    assert!(
        up.contains(&RunStatus::NoStop) && up.contains(&RunStatus::Open) && up.contains(&RunStatus::Closed),
        "{up:?}"
    );
    let rank = |s: &RunStatus| match s {
        RunStatus::NoStart => 0,
        RunStatus::NoStop => 1,
        RunStatus::Open => 2,
        RunStatus::Closed => 3,
    };
    assert!(up.windows(2).all(|w| rank(&w[0]) <= rank(&w[1])), "worst first: {up:?}");
    let mut down = statuses(true);
    down.reverse();
    assert_eq!(down.iter().map(rank).collect::<Vec<_>>(), up.iter().map(rank).collect::<Vec<_>>());
}

#[test]
fn the_runs_table_sorts_filters_and_pages() {
    let dir = Dir::new("table");
    dir.write("worker.txt", &shared_file());
    let ws = open(&dir);
    let durations = |req: RunsRequest| ws.runs(&req).runs.iter().map(|a| (a.to - a.from) / MIN).collect::<Vec<_>>();
    assert_eq!(
        durations(RunsRequest {
            sort: RunSort::Duration,
            descending: true,
            ..Default::default()
        }),
        vec![30, 15, 5, 2]
    );
    assert_eq!(
        durations(RunsRequest {
            sort: RunSort::Stop,
            ..Default::default()
        }),
        vec![15, 30, 2, 5]
    );
    assert_eq!(
        durations(RunsRequest {
            sort: RunSort::Status,
            ..Default::default()
        }),
        vec![2, 5, 30, 15]
    );
    assert_eq!(
        durations(RunsRequest {
            sort: RunSort::Lane,
            ..Default::default()
        }),
        vec![30, 5, 15, 2]
    );
    assert_eq!(
        durations(RunsRequest {
            statuses: vec![RunStatus::Open],
            ..Default::default()
        }),
        vec![2, 5]
    );
    assert_eq!(
        durations(RunsRequest {
            instances: vec!["2210".into()],
            ..Default::default()
        }),
        vec![15, 2]
    );
    assert!(durations(RunsRequest {
        kind: Some("other".into()),
        ..Default::default()
    })
    .is_empty());
    let page = ws.runs(&RunsRequest {
        offset: 1,
        limit: Some(2),
        ..Default::default()
    });
    assert_eq!((page.total, page.offset, page.runs.len()), (4, 1, 2));
    let late = ws.runs(&RunsRequest {
        offset: 99,
        ..Default::default()
    });
    assert_eq!((late.offset, late.runs.len()), (4, 0));
    // A time range keeps the summaries to the runs overlapping it.
    let from = ws.records()[0].ts.unwrap();
    let early = ws.runs(&RunsRequest {
        from: Some(from),
        to: Some(from + 3 * MIN),
        ..Default::default()
    });
    assert_eq!(early.summaries[0].runs, 1);
    assert_eq!(ws.span_summaries()[0].runs, 4);
}

#[test]
fn rules_per_field_and_switched_off_rules() {
    let dir = Dir::new("field");
    dir.write("worker.txt", &shared_file());
    let profile = Profile::from_toml(
        "[instance]\nkey = ['MachineName']\n[[span]]\nkind='proc'\nlabel='Process'\nstart='start'\nstop='stop'\nper='field:ProcessId'\nmax_duration='20m'\n[[span]]\nkind='off'\nstart='start'\nstop='stop'\nenabled=false\n",
    )
    .unwrap();
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(std::slice::from_ref(&dir.0)).unwrap();
    let reply = ws.runs(&RunsRequest::default());
    // One lane (the machine), runs paired per process id.
    let got: Vec<(String, String, RunStatus, bool)> = reply
        .runs
        .iter()
        .map(|a| {
            let r = a.run.as_ref().unwrap();
            (
                a.instance.clone().unwrap(),
                r.group.as_ref().unwrap().value.clone(),
                r.status,
                r.too_long,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("WORKER01".into(), "2208".into(), RunStatus::Closed, true),
            ("WORKER01".into(), "2210".into(), RunStatus::Closed, false),
            ("WORKER01".into(), "2210".into(), RunStatus::Open, false),
            ("WORKER01".into(), "2208".into(), RunStatus::Open, false),
        ]
    );
    assert_eq!(reply.summaries.len(), 2);
    assert_eq!((reply.summaries[0].too_long, reply.summaries[0].anomalies()), (1, 1));
    assert!(!reply.summaries[1].enabled && reply.summaries[1].runs == 0);
    // The evidence of a run on a shared lane is its process.
    let page = ws
        .page(&imt_core::workspace::PageRequest {
            query: reply.runs[1].evidence_query.clone(),
            offset: 0,
            limit: 50,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert_eq!(page.total, 3);
}

#[test]
fn preview_of_a_draft_with_patterns() {
    let dir = Dir::new("preview");
    dir.write("worker.txt", &shared_file());
    let ws = open(&dir);
    let req: SpanPreviewRequest = serde_json::from_value(serde_json::json!({
        "draft": {
            "kind": "batch",
            "label": "Batch",
            "start": { "type": "pattern", "pattern": "processing batch" },
            "stop": { "type": "pattern", "pattern": "worker service stopped" },
            "per": "instance"
        }
    }))
    .unwrap();
    let p = ws.preview_spans(&req);
    assert!(p.errors.is_empty(), "{:?}", p.errors);
    let s = p.summary.unwrap();
    // 2208: batch, batch (restart → no stop), …; 2210: batch → stop, batch (open).
    assert_eq!((s.runs, s.closed, s.no_stop, s.open), (4, 1, 1, 2), "{s:?}");
    assert_eq!(p.runs.len(), 4);
    assert_eq!(p.anomalies.len(), 1);
    assert_eq!(p.plan.unwrap().markers.len(), 2);
    // Errors come back by field, without a summary.
    let bad: SpanPreviewRequest = serde_json::from_value(serde_json::json!({
        "draft": { "kind": "x", "start": { "type": "pattern", "pattern": "(" }, "stop": { "type": "markers", "kinds": ["nope"] } }
    }))
    .unwrap();
    let p = ws.preview_spans(&bad);
    assert_eq!(json(&p.errors)[0]["field"], "start");
    assert_eq!(json(&p.errors)[1]["field"], "stop");
    assert!(p.summary.is_none() && p.plan.is_none());
}

fn runs_json(ws: &Workspace) -> serde_json::Value {
    json((ws.runs(&RunsRequest::default()), ws.annotations(&AnnotationsRequest::default())))
}

#[test]
fn live_tail_appends_equal_a_full_recompute() {
    let dir = Dir::new("tail");
    let file = dir.write("worker.txt", &shared_file());
    let mut ws = open(&dir);
    let before = ws.runs(&RunsRequest::default());
    assert_eq!(before.total, 4);
    // The stop of 2208's second run, more work, and a stop without start for a new process.
    let more = [
        line(52, 2208, "Application stopped"),
        line(53, 2210, "processing batch"),
        line(54, 2299, "Worker service stopped"),
    ]
    .concat();
    let path = dir.append("worker.txt", &more);
    assert_eq!(path, file);
    let report = ws.refresh_tail(&[path]).unwrap();
    assert_eq!(report.appended, 3);
    assert!(report.ids_changed_from.is_none(), "a pure append");
    let incremental = runs_json(&ws);

    let fresh = open(&dir);
    assert_eq!(incremental, runs_json(&fresh));
    let after = ws.runs(&RunsRequest::default());
    let statuses: Vec<RunStatus> = after.runs.iter().map(|a| a.run.as_ref().unwrap().status).collect();
    assert_eq!(
        statuses,
        vec![
            RunStatus::Closed,
            RunStatus::Closed,
            RunStatus::Open,
            RunStatus::Closed,
            RunStatus::NoStart
        ]
    );
    assert_eq!(after.summaries[0].no_start, 1);
}

#[test]
fn removing_a_source_and_changing_the_profile_recompute() {
    let dir = Dir::new("recompute");
    dir.write("a.txt", &shared_file());
    dir.write("b.txt", &line(1, 3000, "Application started"));
    let mut ws = open(&dir);
    assert_eq!(ws.runs(&RunsRequest::default()).total, 5);
    let id = ws.summary().sources.iter().find(|s| s.name == "b.txt").unwrap().id;
    ws.remove_source(id);
    assert_eq!(ws.runs(&RunsRequest::default()).total, 4);
    ws.set_profile(Profile::from_toml("span = []").unwrap()).unwrap();
    assert_eq!(ws.runs(&RunsRequest::default()).total, 0);
    assert!(ws.annotations(&AnnotationsRequest::default()).iter().all(|a| a.run.is_none()));
    ws.clear();
    assert_eq!(ws.runs(&RunsRequest::default()).total, 0);
}

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn sample_logs() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(samples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| n != "profiling"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn samples_show_the_batch_run_and_the_crashed_instance() {
    let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
    let mut ws = Workspace::new(Profile::from_toml(&text).unwrap()).unwrap();
    ws.open(&sample_logs()).unwrap();
    let reply = ws.runs(&RunsRequest::default());
    let lane = |key: &str| -> Vec<RunStatus> {
        reply
            .runs
            .iter()
            .filter(|a| a.instance.as_deref() == Some(key))
            .map(|a| a.run.as_ref().unwrap().status)
            .collect()
    };
    // The nightly reindex worker started and stopped; the crashed web instance ran until "Host terminated"
    // (its "Application started. Press Ctrl+C to shut down." names a stop too, but is a start); the gateway still runs.
    assert!(lane("WORKER01 · 2210").contains(&RunStatus::Closed), "{:?}", lanes(&reply));
    assert_eq!(lane("WEB02 · 4120"), vec![RunStatus::Closed], "{:?}", lanes(&reply));
    let crashed = reply.runs.iter().find(|a| a.instance.as_deref() == Some("WEB02 · 4120")).unwrap();
    assert_eq!(
        crashed.run.as_ref().unwrap().stop.as_ref().unwrap().message,
        "Host terminated unexpectedly"
    );
    assert_eq!(lane("GATEWAY · 1880"), vec![RunStatus::Open]);
    let closed = reply
        .runs
        .iter()
        .find(|a| a.instance.as_deref() == Some("WORKER01 · 2210"))
        .unwrap();
    assert_eq!(closed.run.as_ref().unwrap().duration_ms, 45 * MIN);
    assert!(reply.summaries[0].runs >= 5);
}

fn lanes(reply: &imt_core::spans::RunsReply) -> Vec<(Option<String>, RunStatus)> {
    reply
        .runs
        .iter()
        .map(|a| (a.instance.clone(), a.run.as_ref().unwrap().status))
        .collect()
}
