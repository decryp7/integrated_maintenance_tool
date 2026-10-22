//! Error outliers (ADR 0045) through the `Query` port of the in-memory workspace: the planted cause is found,
//! requests are validated with a message, and empty sets are not errors. (The DuckDB store's own tests check that it
//! gives the same answer.)

use imt_core::analysis::outliers::{error_outliers, OutlierRequest, DEFAULT_SUBSET};
use imt_core::ports::{PageQuery, Query, StoreError};
use imt_core::query::Query as Filter;
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace};

/// 600 events on two hosts and two builds; every error comes from host B on build 3.1 while host A and B otherwise
/// behave alike.
fn store() -> (WorkspaceStore, std::path::PathBuf) {
    let mut text = String::new();
    for i in 0..600 {
        let (host, build) = if i % 2 == 0 { ("A", "3.0") } else { ("B", "3.1") };
        let failing = host == "B" && i % 10 == 1;
        let level = if failing { "ERR" } else { "INF" };
        let message = if failing { "Request failed" } else { "Request served" };
        let (m, s) = (i / 60, i % 60);
        text.push_str(&format!(
            "2026-10-14 09:{m:02}:{s:02}.000 +00:00 [{level}] {message} {{\"Host\": \"{host}\", \"Build\": \"{build}\", \"Region\": \"r{}\"}}\n",
            i % 3
        ));
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("imt-core-outliers-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("svc.log");
    std::fs::write(&path, text).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&path)).unwrap();
    (WorkspaceStore::new(ws), dir)
}

#[test]
fn the_planted_cause_is_found_with_its_evidence() {
    let (store, dir) = store();
    let report = error_outliers(&store, &OutlierRequest::default()).unwrap();
    let r = &report.result;
    assert_eq!(r.subset, DEFAULT_SUBSET);
    assert_eq!((r.subset_total, r.rest_total), (60, 540));
    let top: Vec<(&str, &str)> = r.outliers.iter().map(|o| (o.field.as_str(), o.value.as_str())).collect();
    assert!(top.contains(&("Host", "B")) && top.contains(&("Build", "3.1")), "{top:?}");
    assert!(
        !top.iter().any(|(f, _)| *f == "Region"),
        "an unrelated field is not an outlier: {top:?}"
    );
    let host = r.outliers.iter().find(|o| o.field == "Host").unwrap();
    assert_eq!((host.subset_count, host.rest_count), (60, 240));
    assert!(host.lift > 1.5 && host.p_value < 0.001);
    assert_eq!(host.sample_ids.len(), 5);
    assert!(host.query.contains("Host = \"B\""));
    // The evidence query selects exactly the subset events with the value.
    let views = store
        .page(&PageQuery {
            query: Filter {
                ql: host.query.clone(),
                ..Filter::default()
            },
            offset: 0,
            limit: 100,
            descending: false,
            anchor: None,
        })
        .unwrap();
    assert_eq!(views.total, 60);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_chosen_subset_fields_and_limit_are_honoured() {
    let (store, dir) = store();
    let req = OutlierRequest {
        subset: "Host = \"B\"".into(),
        fields: vec!["Build".into(), "Region".into(), "@level".into()],
        limit: 5,
        ..OutlierRequest::default()
    };
    let report = error_outliers(&store, &req).unwrap();
    let found: Vec<_> = report
        .result
        .outliers
        .iter()
        .map(|o| (o.field.as_str(), o.value.as_str()))
        .collect();
    assert!(found.contains(&("Build", "3.1")), "{found:?}");
    assert_eq!(report.result.fields_checked, 3);
    assert!(report.result.outliers.len() <= 5);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_requests_fail_with_a_message_and_empty_sets_are_not_an_error() {
    let (ws, dir) = store();
    for store in [&ws as &dyn Query] {
        let e = error_outliers(
            store,
            &OutlierRequest {
                subset: "level = = 1".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(&e, StoreError::Invalid(m) if m.contains("subset")), "{e:?}");
        let e = error_outliers(
            store,
            &OutlierRequest {
                subset: "a | stats count() as n".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(e, StoreError::Invalid(_)));
        let e = error_outliers(
            store,
            &OutlierRequest {
                query: Filter {
                    ql: "x | stats count() as n".into(),
                    ..Filter::default()
                },
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(e, StoreError::Invalid(_)));
        let e = error_outliers(
            store,
            &OutlierRequest {
                fields: vec!["bad field".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(e, StoreError::Invalid(_)));
        // No events in the subset: an empty report, not an error.
        let none = error_outliers(
            store,
            &OutlierRequest {
                subset: "@level = fatal".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(none.result.outliers.is_empty() && none.result.subset_total == 0);
        // Everything is in the subset: nothing to compare with.
        let all = error_outliers(
            store,
            &OutlierRequest {
                subset: "@level >= verbose".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(all.result.outliers.is_empty() && all.result.rest_total == 0);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cancelled_run_stops() {
    let (store, dir) = store();
    let control = imt_core::ql::QueryControl::new();
    control.cancel();
    let e = control.scope(|| error_outliers(&store, &OutlierRequest::default())).unwrap_err();
    assert_eq!(e, StoreError::Cancelled);
    let _ = std::fs::remove_dir_all(dir);
}
