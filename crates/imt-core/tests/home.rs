//! The Home dashboard's summary (`imt_core::home::home_summary`) over the in-memory store ports, on the generated
//! incident in /samples: its counts agree with the store's own queries, the history covers every timed event, and
//! an empty store gives an empty, timeless summary.

use imt_core::api::QueryRequest;
use imt_core::home::{home_summary, HomeRequest, MAX_HOME_BUCKETS, MAX_HOME_MARKS, MIN_HOME_BUCKETS};
use imt_core::ports::{Analysis as _, Query as _};
use imt_core::query::Query;
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace};
use std::path::PathBuf;
use std::sync::OnceLock;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn store() -> &'static WorkspaceStore {
    static S: OnceLock<WorkspaceStore> = OnceLock::new();
    S.get_or_init(|| {
        let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
        let mut ws = Workspace::new(Profile::from_toml(&text).unwrap()).unwrap();
        ws.open(&[samples().join("serilog-clef")]).unwrap();
        WorkspaceStore::new(ws)
    })
}

#[test]
fn counts_agree_with_the_store_queries() {
    let s = store();
    let home = home_summary(s, &HomeRequest::default()).unwrap();
    let summary = s.summary().unwrap();
    assert!(home.events > 0);
    assert_eq!(home.events, summary.total);
    assert_eq!(home.sources, summary.sources.len());
    assert_eq!((home.from, home.to), (summary.from, summary.to));
    let all = QueryRequest { query: Query::default() };
    let anomalies = s.anomalies(&all).unwrap().result;
    assert_eq!(home.anomaly_count, anomalies.len());
    assert_eq!(home.anomalies.len(), anomalies.len().min(MAX_HOME_MARKS));
    assert!(home.anomalies.windows(2).all(|w| w[0].from <= w[1].from), "anomalies by start time");
    let groups = s.exception_groups(&all).unwrap().result;
    assert_eq!(home.exception_groups, groups.len());
    assert_eq!(home.exception_events, groups.iter().map(|g| g.count).sum::<usize>());
}

#[test]
fn history_covers_every_timed_event_with_the_trends() {
    let s = store();
    let home = home_summary(s, &HomeRequest::default()).unwrap();
    let summary = s.summary().unwrap();
    assert_eq!(home.histogram.len(), 60);
    let timed: u64 = home.histogram.iter().flatten().map(|&n| u64::from(n)).sum();
    assert_eq!(timed, (summary.total - summary.untimed) as u64);
    assert!(home.bucket_ms > 0.0);
    let errors: u64 = home.error_trend.iter().map(|&n| u64::from(n)).sum();
    let warnings: u64 = home.warning_trend.iter().map(|&n| u64::from(n)).sum();
    assert_eq!(home.error_trend.len(), home.histogram.len());
    assert_eq!(home.warning_trend.len(), home.histogram.len());
    assert!(errors <= home.errors as u64 && warnings <= home.warnings as u64);
    assert!(home.markers.windows(2).all(|w| w[0].ts <= w[1].ts), "markers by time");
}

#[test]
fn bucket_counts_are_clamped() {
    let s = store();
    let few = home_summary(
        s,
        &HomeRequest {
            query: Query::default(),
            buckets: 0,
            analyses: true,
        },
    )
    .unwrap();
    assert_eq!(few.histogram.len(), MIN_HOME_BUCKETS);
    let many = home_summary(
        s,
        &HomeRequest {
            query: Query::default(),
            buckets: 100_000,
            analyses: true,
        },
    )
    .unwrap();
    assert_eq!(many.histogram.len(), MAX_HOME_BUCKETS);
}

#[test]
fn an_empty_store_has_no_history() {
    let s = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
    let home = home_summary(&s, &HomeRequest::default()).unwrap();
    assert_eq!((home.sources, home.events, home.anomaly_count, home.exception_groups), (0, 0, 0, 0));
    assert_eq!((home.from, home.to), (None, None));
    assert!(home.histogram.is_empty() && home.error_trend.is_empty() && home.markers.is_empty());
    assert_eq!(home.bucket_ms, 0.0);
}

#[test]
fn an_invalid_query_is_a_typed_error() {
    let s = store();
    let req = HomeRequest {
        query: Query {
            ql: "level = = (".into(),
            ..Default::default()
        },
        buckets: 60,
        analyses: true,
    };
    assert!(home_summary(s, &req).is_err());
}

#[test]
fn without_analyses_the_counts_and_history_come_back_and_the_slow_parts_are_skipped() {
    let s = store();
    let full = home_summary(s, &HomeRequest::default()).unwrap();
    let fast = home_summary(
        s,
        &HomeRequest {
            analyses: false,
            ..HomeRequest::default()
        },
    )
    .unwrap();
    assert!(full.analysed && !fast.analysed);
    assert_eq!((fast.events, fast.errors, fast.warnings), (full.events, full.errors, full.warnings));
    assert_eq!(fast.histogram, full.histogram);
    assert_eq!(fast.markers, full.markers);
    assert_eq!((fast.anomaly_count, fast.exception_groups, fast.exception_events), (0, 0, 0));
    assert!(fast.anomalies.is_empty());
    assert_eq!(fast.coverage.matched, 0, "no analysis ran, so nothing was covered");
}
