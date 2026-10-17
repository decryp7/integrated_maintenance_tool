//! Insights over the generated incident in /samples, through the LogStore
//! abstraction the UI uses.

use imt_core::analysis::AnomalyKind;
use imt_core::api::{
    CompareRequest, CompletionRequest, CorrelateRequest, FieldStatsRequest, MomentRequest, PageRequest, QueryRequest, RequestsRequest,
    TimelineRequest, TraceRequest,
};
use imt_core::ql::QlResult;
use imt_core::query::Query;
use imt_core::{LogStore, Profile, Workspace};
use std::path::PathBuf;
use std::sync::OnceLock;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Loaded once: the samples take a moment to import.
fn store() -> &'static (dyn LogStore + 'static) {
    static STORE: OnceLock<Box<dyn LogStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
            let mut ws: Box<dyn LogStore> = Box::new(Workspace::new(Profile::from_toml(&text).unwrap()).unwrap());
            ws.open(&[samples()]).unwrap();
            ws
        })
        .as_ref()
}

fn ms(h: u32, m: u32, s: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(2026, 10, 14)
        .unwrap()
        .and_hms_opt(h, m, s)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

fn q(ql: &str) -> Query {
    Query {
        ql: ql.into(),
        ..Default::default()
    }
}

#[test]
fn query_language_filters_everything_and_runs_stats() {
    let s = store();
    let errors = s
        .page(&PageRequest {
            query: q("level >= error and MachineName = WEB02"),
            offset: 0,
            limit: 5,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert!(errors.total > 0);
    assert!(errors
        .rows
        .iter()
        .all(|r| r.fields["MachineName"] == "WEB02" && r.level >= imt_core::model::Level::Error));
    let tl = s
        .timeline(&TimelineRequest {
            query: q("level = fatal"),
            buckets: 50,
            always_show_lifecycle: false,
        })
        .unwrap();
    assert_eq!(tl.histogram.iter().map(|b| b.iter().sum::<u32>()).sum::<u32>(), 1);

    let table = s
        .run_query(&QueryRequest {
            query: q("level >= warning | stats count() as n by MachineName | sort n desc"),
        })
        .unwrap();
    let QlResult::Table { columns, rows } = table else {
        panic!("expected a table")
    };
    assert_eq!(columns, ["MachineName", "n"]);
    assert!(!rows.is_empty());
    let events = s
        .run_query(&QueryRequest {
            query: q("RequestPath ~ \"capture\" | head 3"),
        })
        .unwrap();
    assert!(matches!(events, QlResult::Events { ids, .. } if ids.len() == 3));

    let err = s.run_query(&QueryRequest { query: q("level >") }).unwrap_err();
    assert!(err.offset > 0);
    assert!(s.check_query("level >= warning").is_none());
    assert!(s.check_query("level >=").is_some());
    // Did you mean: an unknown field comes back with the closest names in the samples.
    let e = s.check_query("level >= error and machinename = WEB01").unwrap();
    assert_eq!(e.message, "unknown field \"machinename\"");
    assert_eq!(&"level >= error and machinename = WEB01"[e.offset..e.offset + e.len], "machinename");
    assert_eq!(e.suggestions.first().map(String::as_str), Some("MachineName"), "{e:?}");
    assert!(serde_json::to_value(&e).unwrap()["suggestions"].is_array());
    assert!(s.check_query("MachineName = WEB01 | stats count() by MachineName").is_none());
    assert!(serde_json::to_value(s.check_query("level >=").unwrap())
        .unwrap()
        .get("suggestions")
        .is_none());
    let bad = s.page(&PageRequest {
        query: q("(("),
        offset: 0,
        limit: 1,
        descending: false,
        anchor: None,
    });
    assert!(bad.is_err());
    let c = s.complete_query(&CompletionRequest {
        text: "Machi".into(),
        cursor: 5,
    });
    assert!(c.iter().any(|c| c.label == "MachineName"));
}

#[test]
fn patterns_exceptions_and_anomalies() {
    let s = store();
    let patterns = s
        .patterns(&QueryRequest {
            query: q("MachineName = WEB02"),
        })
        .unwrap();
    assert!(
        patterns.iter().any(|p| p.template.contains("Failed to capture payment")),
        "{:?}",
        patterns.iter().take(5).map(|p| &p.template).collect::<Vec<_>>()
    );

    let groups = s.exception_groups(&QueryRequest { query: Query::default() }).unwrap();
    let timeout = groups
        .iter()
        .find(|g| g.type_name.contains("TimeoutException"))
        .expect("timeout group");
    assert!(timeout.count > 1);
    assert!(timeout.instances.iter().any(|(i, _)| i.contains("WEB02")));

    let anomalies = s.anomalies(&QueryRequest { query: Query::default() }).unwrap();
    let crash = anomalies.iter().find(|a| matches!(a.kind, AnomalyKind::Crash)).expect("crash");
    assert!(crash.instance.as_deref().unwrap().contains("4120"));
    // Cached: the same query returns the same result.
    assert_eq!(
        s.anomalies(&QueryRequest { query: Query::default() }).unwrap().len(),
        anomalies.len()
    );
}

#[test]
fn compare_stats_correlate_traces_and_requests() {
    let s = store();
    let before = Query {
        from: Some(ms(9, 0, 0)),
        to: Some(ms(10, 0, 0)),
        ..Default::default()
    };
    let during = Query {
        from: Some(ms(10, 30, 0)),
        to: Some(ms(10, 41, 30)),
        ..Default::default()
    };
    let cmp = s.compare(&CompareRequest { a: before, b: during }).unwrap();
    assert!(cmp.b.per_minute > 0.0 && !cmp.summary.is_empty());

    let cpu = s
        .field_stats(&FieldStatsRequest {
            query: Query::default(),
            field: "CpuPercent".into(),
            buckets: 10,
        })
        .unwrap();
    assert!(cpu.numeric.as_ref().unwrap().max > 90.0);

    let any = s
        .page(&PageRequest {
            query: q("exists(CorrelationId)"),
            offset: 0,
            limit: 1,
            descending: false,
            anchor: None,
        })
        .unwrap();
    let id = any.rows[0].fields["CorrelationId"].as_str().unwrap().to_string();
    let ids = s.correlate(&CorrelateRequest {
        field: "CorrelationId".into(),
        value: id,
    });
    assert!(!ids.is_empty());

    let (traces, rows) = s
        .traces(&TraceRequest {
            query: Query::default(),
            trace_id: String::new(),
        })
        .unwrap();
    assert!(rows.is_empty());
    if let Some(t) = traces.first() {
        let (_, waterfall) = s
            .traces(&TraceRequest {
                query: Query::default(),
                trace_id: t.trace_id.clone(),
            })
            .unwrap();
        assert!(!waterfall.is_empty());
    }
    let reqs = s
        .requests(&RequestsRequest {
            query: Query::default(),
            id_field: "CorrelationId".into(),
        })
        .unwrap();
    assert!(!reqs.is_empty() && reqs.len() <= 500);
    let ordered = reqs.windows(2).all(|w| (w[0].last - w[0].first) >= (w[1].last - w[1].first));
    assert!(ordered);
}

#[test]
fn moment_view_merges_metrics_activity_logs_and_anomalies() {
    let s = store();
    let m = s
        .moment(&MomentRequest {
            query: Query::default(),
            from: ms(10, 39, 0),
            to: ms(10, 41, 30),
        })
        .unwrap();
    assert!(m.total > 0);
    let web02 = m
        .instances
        .iter()
        .find(|i| i.key.contains("WEB02") && i.key.contains("4120"))
        .expect("web02 instance");
    assert!(web02.errors > 0 && web02.first_error.is_some() && !web02.patterns.is_empty());
    assert!(
        m.instances.first().unwrap().errors >= m.instances.last().unwrap().errors,
        "busiest-with-errors first"
    );

    // Metrics: the monitor fields, compared with the preceding window.
    let cpu = m
        .metrics
        .iter()
        .find(|x| x.metric == "CpuPercent" && x.series.contains("4120"))
        .expect("cpu of 4120");
    assert!(cpu.max >= cpu.avg && cpu.avg >= cpu.min);
    assert!(cpu.previous_avg.is_some());
    // PerfMon long-format counters from the CSV sample count too.
    assert!(
        m.metrics
            .iter()
            .any(|x| x.metric.contains("Processor") || x.metric.contains("Memory")),
        "{:?}",
        m.metrics.iter().map(|x| &x.metric).collect::<Vec<_>>()
    );

    // Activity: RequestPath acts as the action, ClientIp/users absent in the samples.
    assert!(!m.activity.is_empty());
    assert!(m.activity.iter().any(|a| a.action.contains("/api/")));
    assert!(m
        .anomalies
        .iter()
        .any(|a| matches!(a.kind, AnomalyKind::Crash | AnomalyKind::ErrorSpike)));

    // Reversed bounds are normalised; an empty window is valid.
    let empty = s
        .moment(&MomentRequest {
            query: Query::default(),
            from: ms(3, 0, 1),
            to: ms(3, 0, 0),
        })
        .unwrap();
    assert_eq!((empty.total, empty.from < empty.to), (0, true));
}

#[test]
fn parse_preview_uses_the_real_engine() {
    use imt_core::api::ParsePreviewRequest;
    let s = store();
    let sample = "[10:00:01 WRN] [WEB09] Disk almost full\n[10:00:02 INF] [WEB09] Application started\n";
    let toml = "[parse]\noutput_templates = ['[{Timestamp:HH:mm:ss} {Level:u3}] [{MachineName}] {Message:lj}{NewLine}{Exception}']\n[[pipeline]]\nstep = 'set'\nfield = 'Team'\nvalue = 'ops'";
    let p = s.preview_parse(&ParsePreviewRequest {
        toml: toml.into(),
        sample: sample.into(),
        file_name: "x.log".into(),
        limit: 1,
    });
    assert_eq!(p.error, None);
    assert_eq!((p.importer.as_deref(), p.total, p.records.len()), (Some("text"), 2, 1));
    assert_eq!(p.records[0].fields["MachineName"], "WEB09");
    assert_eq!(p.records[0].fields["Team"], "ops");
    assert_eq!(p.records[0].level, imt_core::model::Level::Warning);

    let nothing = s.preview_parse(&ParsePreviewRequest {
        toml: String::new(),
        sample: "plain words".into(),
        file_name: String::new(),
        limit: 5,
    });
    assert!(nothing.error.unwrap().contains("no importer"));
    let bad = s.preview_parse(&ParsePreviewRequest {
        toml: "[parse".into(),
        sample: String::new(),
        file_name: String::new(),
        limit: 5,
    });
    assert!(bad.error.is_some());
    let start = s.preview_parse(&ParsePreviewRequest {
        toml: String::new(),
        sample: "2026-10-14 10:00:00 INFO Application started\n".into(),
        file_name: String::new(),
        limit: 5,
    });
    assert_eq!(start.records[0].markers, ["start"]);
}

#[test]
fn overview_lists_findings_most_severe_first() {
    use imt_core::api::{FindingAction, Severity};
    let s = store();
    let findings = s.overview(&QueryRequest { query: Query::default() }).unwrap();
    assert_eq!(findings[0].severity, Severity::Critical);
    assert!(
        findings[0].title == "Crash" && findings[0].detail.contains("4120"),
        "{:?}",
        findings[0]
    );
    assert!(matches!(findings[0].action, Some(FindingAction::Zoom { .. })));
    assert!(findings.windows(2).all(|w| w[0].severity <= w[1].severity));
    assert!(
        findings.iter().any(|f| f.title.starts_with("TimeoutException ×")),
        "{:?}",
        findings.iter().map(|f| &f.title).collect::<Vec<_>>()
    );
    assert!(findings
        .iter()
        .any(|f| f.title == "File not opened" && matches!(f.action, Some(FindingAction::SetupFormat { .. }))));
    assert!(findings.iter().any(|f| f.title == "Memory growth"));
    use imt_core::api::FindingKind;
    assert_eq!(findings[0].kind, FindingKind::Crash);
    assert!(findings
        .iter()
        .any(|f| f.kind == FindingKind::Exception && f.title.starts_with("TimeoutException")));
    assert!(findings.iter().any(|f| f.kind == FindingKind::FileNotOpened));
    assert!(findings.iter().any(|f| f.kind == FindingKind::MemoryGrowth));
    let json = serde_json::to_value(&findings[0]).unwrap();
    assert_eq!(json["kind"], "crash");
    assert_eq!(json["action"]["kind"], "zoom");
}
