//! The health check's file findings, traces, PerfMon metric series and query
//! errors, on small purpose-built folders.

use imt_core::api::{Finding, FindingAction, MomentRequest, QueryRequest, Severity, TraceRequest};
use imt_core::query::Query;
use imt_core::{LogStore, Profile, Workspace};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A temporary folder removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("imt-health-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&p).unwrap();
        // Opened paths are canonical (e.g. /private/var on macOS).
        TempDir(p.canonicalize().unwrap())
    }

    fn file(&self, name: &str, lines: &[String]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, lines.concat()).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn line(json: &str) -> String {
    format!("{json}\n")
}

fn ms(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
}

fn open(profile: Profile, d: &TempDir) -> Workspace {
    let mut ws = Workspace::new(profile).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    ws
}

fn overview(ws: &Workspace) -> Vec<Finding> {
    let store: &dyn LogStore = ws;
    store.overview(&QueryRequest { query: Query::default() }).unwrap()
}

#[test]
fn health_check_flags_unreadable_lines_missing_timestamps_and_dropped_events() {
    let d = TempDir::new();
    let mut garbled = vec![line(r#"{"@t":"2026-10-14T10:00:00Z","@mt":"ok"}"#)];
    garbled.extend((0..3).map(|i| line(&format!("not json {i}"))));
    let garbled_path = d.file("garbled.clef", &garbled);
    let untimed_path = d.file(
        "untimed.clef",
        &[line(r#"{"@mt":"no clock here"}"#), line(r#"{"@mt":"still none"}"#)],
    );
    d.file(
        "noisy.clef",
        &[
            line(r#"{"@t":"2026-10-14T10:00:01Z","@mt":"noise"}"#),
            line(r#"{"@t":"2026-10-14T10:00:02Z","@mt":"noise"}"#),
            line(r#"{"@t":"2026-10-14T10:00:03Z","@mt":"signal"}"#),
        ],
    );
    let profile = Profile::from_toml("[[pipeline]]\nstep = 'drop'\nwhen = [{ field = '@message', op = 'eq', value = 'noise' }]").unwrap();
    let ws = open(profile, &d);
    let findings = overview(&ws);
    let find = |title: &str| {
        findings
            .iter()
            .find(|f| f.title == title)
            .unwrap_or_else(|| panic!("no {title:?} in {findings:#?}"))
    };

    let unreadable = find("Lines not understood");
    assert_eq!(unreadable.severity, Severity::Warning);
    assert_eq!(unreadable.detail, "garbled.clef: 75% of the lines could not be read as events");
    assert_eq!(
        unreadable.action,
        Some(FindingAction::SetupFormat {
            path: garbled_path.to_string_lossy().into_owned()
        })
    );

    let untimed = find("No timestamps");
    assert_eq!(untimed.severity, Severity::Info);
    assert!(
        untimed.detail.starts_with("untimed.clef has no recognisable timestamps"),
        "{}",
        untimed.detail
    );
    assert_eq!(
        untimed.action,
        Some(FindingAction::SetupFormat {
            path: untimed_path.to_string_lossy().into_owned()
        })
    );

    let dropped = find("Events removed by the pipeline");
    assert!(dropped.detail.starts_with("2 events were dropped"), "{}", dropped.detail);
    assert_eq!(dropped.action, Some(FindingAction::OpenPanel { panel: "pipeline".into() }));

    // Most severe first.
    assert!(findings.windows(2).all(|w| w[0].severity <= w[1].severity));
}

#[test]
fn a_clean_folder_has_no_file_findings() {
    let d = TempDir::new();
    d.file("app.clef", &[line(r#"{"@t":"2026-10-14T10:00:00Z","@mt":"fine"}"#)]);
    let ws = open(Profile::default_profile(), &d);
    assert!(overview(&ws).is_empty());
}

#[test]
fn traces_are_listed_longest_first_and_expand_into_a_waterfall() {
    let d = TempDir::new();
    d.file(
        "trace.clef",
        &[
            line(r#"{"@t":"2026-10-14T10:00:00.000Z","@mt":"GET /orders","TraceId":"t1","SpanId":"root","RequestPath":"/orders"}"#),
            line(r#"{"@t":"2026-10-14T10:00:00.100Z","@mt":"query db","TraceId":"t1","SpanId":"db","ParentSpanId":"root"}"#),
            line(r#"{"@t":"2026-10-14T10:00:00.400Z","@l":"Error","@mt":"db timeout","TraceId":"t1","SpanId":"db","ParentSpanId":"root"}"#),
            line(r#"{"@t":"2026-10-14T10:00:01.000Z","@mt":"GET /orders done","TraceId":"t1","SpanId":"root"}"#),
            // A trace whose only span names a parent that was never logged.
            line(r#"{"@t":"2026-10-14T10:00:02.000Z","@mt":"orphan work","TraceId":"t2","SpanId":"x","ParentSpanId":"gone"}"#),
            line(r#"{"@t":"2026-10-14T10:00:02.050Z","@mt":"orphan done","TraceId":"t2","SpanId":"x"}"#),
        ],
    );
    let ws = open(Profile::default_profile(), &d);
    let store: &dyn LogStore = &ws;
    let (traces, rows) = store
        .traces(&TraceRequest {
            query: Query::default(),
            trace_id: String::new(),
        })
        .unwrap();
    assert!(rows.is_empty());
    let summary: Vec<(&str, usize, u32, f64, &str)> = traces
        .iter()
        .map(|t| (t.trace_id.as_str(), t.spans, t.errors, t.duration_ms, t.root.as_str()))
        .collect();
    assert_eq!(summary, vec![("t1", 2, 1, 1000.0, "/orders"), ("t2", 1, 0, 50.0, "orphan work")]);
    assert_eq!(traces[0].start, ms("2026-10-14T10:00:00Z"));

    let (none, waterfall) = store
        .traces(&TraceRequest {
            query: Query::default(),
            trace_id: "t1".into(),
        })
        .unwrap();
    assert!(none.is_empty());
    let rows: Vec<(&str, u32, i64, u32)> = waterfall
        .iter()
        .map(|r| (r.span_id.as_str(), r.depth, r.start_offset_ms, r.errors))
        .collect();
    assert_eq!(rows, vec![("root", 0, 0, 0), ("db", 1, 100, 1)]);
}

#[test]
fn perfmon_counters_are_series_per_machine_and_instance() {
    let d = TempDir::new();
    let counter = |t: &str, instance: &str, value: f64| {
        line(&format!(
            r#"{{"@t":"2026-10-14T10:00:{t}Z","@mt":"sample","MachineName":"WEB01","Instance":"{instance}","Counter":"% Processor Time","Value":{value}}}"#
        ))
    };
    d.file(
        "perf.clef",
        &[
            counter("01", "w3wp", 10.0),
            counter("11", "w3wp", 30.0),
            counter("12", "w3wp", 50.0),
            counter("13", "", 5.0),
        ],
    );
    let ws = open(Profile::default_profile(), &d);
    let m = ws
        .moment(&MomentRequest {
            query: Query::default(),
            from: ms("2026-10-14T10:00:10Z"),
            to: ms("2026-10-14T10:00:19Z"),
        })
        .unwrap();
    let series: Vec<(&str, &str, usize, f64, Option<f64>)> = m
        .metrics
        .iter()
        .map(|w| (w.series.as_str(), w.metric.as_str(), w.samples, w.avg, w.change_pct))
        .collect();
    assert_eq!(
        series,
        vec![
            ("WEB01 · w3wp", "% Processor Time", 2, 40.0, Some(300.0)),
            ("WEB01", "% Processor Time", 1, 5.0, None),
        ]
    );
}

#[test]
fn analyses_group_by_the_query_and_report_bad_searches() {
    let d = TempDir::new();
    let boom = |t: &str, machine: &str| {
        line(&format!(
            r#"{{"@t":"2026-10-14T10:00:{t}Z","@l":"Error","@mt":"failed","@x":"System.InvalidOperationException: nope\n   at Orders.Save()","MachineName":"{machine}","Env":"prod"}}"#
        ))
    };
    d.file("app.clef", &[boom("01", "WEB01"), boom("02", "WEB02"), boom("03", "WEB02")]);
    let ws = open(Profile::default_profile(), &d);

    // Grouping by a field of the query instead of the profile's instances.
    let groups = ws
        .exception_groups(&QueryRequest {
            query: Query {
                group_by: vec!["Env".into()],
                ..Query::default()
            },
        })
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].count, 3);
    assert_eq!(groups[0].instances.len(), 1, "{:?}", groups[0].instances);
    let by_machine = ws
        .exception_groups(&QueryRequest {
            query: Query {
                group_by: vec!["MachineName".into()],
                ..Query::default()
            },
        })
        .unwrap();
    assert_eq!(by_machine[0].instances.len(), 2);

    // An invalid search regex is reported as a query-language error.
    let err = ws
        .run_query(&QueryRequest {
            query: Query {
                text: "(".into(),
                regex: true,
                ql: "level >= error | stats count()".into(),
                ..Query::default()
            },
        })
        .unwrap_err();
    assert!(err.message.contains("invalid search regex"), "{}", err.message);
    assert_eq!((err.offset, err.len), (0, 0));
}
