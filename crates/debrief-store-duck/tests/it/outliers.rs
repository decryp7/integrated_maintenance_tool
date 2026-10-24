//! Error outliers (ADR 0045): the pure function over the `Query` port gives the same answer from the in-memory
//! `Workspace` and the DuckDB store, and finds the planted cause.

use crate::common;

use common::{store_with, TempDir};
use debrief_core::analysis::outliers::{error_outliers, OutlierRequest, DEFAULT_SUBSET};
use debrief_core::ports::{Query, StoreError};
use debrief_core::query::Query as Filter;
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use debrief_store_duck::DuckStore;
use std::path::PathBuf;

/// 600 events on two hosts and two builds; every error comes from host B on build 3.1 while host A and B otherwise
/// behave alike.
fn corpus(dir: &TempDir) -> Vec<PathBuf> {
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
    let path = dir.0.join("svc.log");
    std::fs::write(&path, text).unwrap();
    vec![path]
}

struct Both {
    ws: WorkspaceStore,
    duck: DuckStore,
    _stores: TempDir,
}

fn both(data: &TempDir) -> Both {
    let paths = corpus(data);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&paths).unwrap();
    let stores = TempDir::new("outliers-store");
    let (duck, _) = store_with(&stores, Profile::default_profile(), &paths).unwrap();
    Both {
        ws: WorkspaceStore::new(ws),
        duck,
        _stores: stores,
    }
}

fn same(a: &debrief_core::analysis::OutlierReport, b: &debrief_core::analysis::OutlierReport) {
    assert_eq!((a.subset_total, a.rest_total), (b.subset_total, b.rest_total));
    let key = |r: &debrief_core::analysis::OutlierReport| -> Vec<(String, String, usize, usize)> {
        r.outliers
            .iter()
            .map(|o| (o.field.clone(), o.value.clone(), o.subset_count, o.rest_count))
            .collect()
    };
    assert_eq!(key(a), key(b));
    for (x, y) in a.outliers.iter().zip(&b.outliers) {
        assert!((x.significance - y.significance).abs() < 1e-9 && (x.lift - y.lift).abs() < 1e-9);
        assert_eq!(x.sample_ids.len(), y.sample_ids.len());
    }
}

#[test]
fn both_stores_find_the_planted_cause() {
    let data = TempDir::new("outliers-data");
    let p = both(&data);
    let req = OutlierRequest::default();
    let ws = error_outliers(&p.ws, &req).unwrap();
    let duck = error_outliers(&p.duck, &req).unwrap();
    same(&ws.result, &duck.result);
    assert_eq!(ws.coverage, duck.coverage);

    let r = &duck.result;
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
    let views = p
        .duck
        .page(&debrief_core::ports::PageQuery {
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
}

#[test]
fn a_chosen_subset_and_fields_work_the_same_in_both_stores() {
    let data = TempDir::new("outliers-data2");
    let p = both(&data);
    let req = OutlierRequest {
        subset: "Host = \"B\"".into(),
        fields: vec!["Build".into(), "Region".into(), "@level".into()],
        limit: 5,
        ..OutlierRequest::default()
    };
    let ws = error_outliers(&p.ws, &req).unwrap();
    let duck = error_outliers(&p.duck, &req).unwrap();
    same(&ws.result, &duck.result);
    let found: Vec<_> = duck.result.outliers.iter().map(|o| (o.field.as_str(), o.value.as_str())).collect();
    assert!(found.contains(&("Build", "3.1")), "{found:?}");
    assert_eq!(duck.result.fields_checked, 3);
}

#[test]
fn bad_requests_fail_with_a_message_and_empty_sets_are_not_an_error() {
    let data = TempDir::new("outliers-data3");
    let p = both(&data);
    for store in [&p.ws as &dyn Query, &p.duck as &dyn Query] {
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
}

#[test]
fn a_cancelled_run_stops() {
    let data = TempDir::new("outliers-data4");
    let p = both(&data);
    let control = debrief_core::ql::QueryControl::new();
    control.cancel();
    let e = control.scope(|| error_outliers(&p.duck, &OutlierRequest::default())).unwrap_err();
    assert_eq!(e, StoreError::Cancelled);
}
