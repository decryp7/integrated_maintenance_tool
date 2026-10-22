//! Error outliers (ADR 0045) through the `Query` port of the in-memory workspace: the planted cause is found,
//! requests are validated with a message, and empty sets are not errors. (The DuckDB store's own tests check that it
//! gives the same answer.)

use imt_core::analysis::outliers::{error_outliers, OutlierRequest, DEFAULT_SUBSET};
mod common;

use common::delegate::Delegating;
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
    store_of(&text)
}

/// A store over one log file with this text.
fn store_of(text: &str) -> (WorkspaceStore, std::path::PathBuf) {
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
    {
        let store: &dyn Query = &ws;
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

/// One log line at event number `i` (ten events a second would collide, so each gets its own second).
fn line(i: usize, level: &str, fields: &str) -> String {
    let secs = i % 86_400;
    format!(
        "2026-10-14 {:02}:{:02}:{:02}.000 +00:00 [{level}] event {i} {{{fields}}}\n",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// 150 errors among 5050 other events, with fields that exercise every limit of the comparison.
fn limits_store() -> (WorkspaceStore, std::path::PathBuf) {
    let mut text = String::new();
    for i in 0..5200 {
        let error = i < 150;
        let level = if error { "ERR" } else { "INF" };
        // Every event has its own `Uniq` (too many values to be a category), errors their own `Rare` value and the
        // others share 1100 values twice or more (so the window's top values leave the errors' out), `Mild` is
        // slightly more common in errors, `Cause` and `Host` mark the errors.
        let rare = if error { format!("z{i}") } else { format!("i{}", i % 1100) };
        // Errors carry one of 100 values once or twice: in the window's top values but too few to prove anything.
        let few = if error { format!("f{}", i % 100) } else { "none".to_string() };
        let mild = if (error && i < 4) || (!error && i < 200) { "x" } else { "y" };
        let host = if error || i % 2 == 0 { "B" } else { "A" };
        let cause = if error { "bad" } else { "ok" };
        text.push_str(&line(
            i,
            level,
            &format!(
                "\"Uniq\":\"u{i}\",\"Rare\":\"{rare}\",\"Few\":\"{few}\",\"Mild\":\"{mild}\",\"Host\":\"{host}\",\"Cause\":\"{cause}\""
            ),
        ));
    }
    store_of(&text)
}

#[test]
fn values_that_cannot_be_judged_or_are_too_weak_are_skipped_and_the_limits_are_reported() {
    let (store, dir) = limits_store();
    let req = OutlierRequest {
        fields: vec![
            "Uniq".into(),
            "Rare".into(),
            "Few".into(),
            "Mild".into(),
            "Cause".into(),
            "Host".into(),
            "Missing".into(),
        ],
        ..OutlierRequest::default()
    };
    let report = error_outliers(&store, &req).unwrap();
    let found: Vec<(&str, &str)> = report
        .result
        .outliers
        .iter()
        .map(|o| (o.field.as_str(), o.value.as_str()))
        .collect();
    assert_eq!(report.result.fields_checked, 7);
    assert!(found.contains(&("Cause", "bad")) && found.contains(&("Host", "B")), "{found:?}");
    // An identifier-like field (5200 values), a field absent from the subset, values too rare to prove anything or
    // beyond the window's top values, and an association that is not significant are all left out.
    for skipped in ["Uniq", "Rare", "Few", "Mild", "Missing"] {
        assert!(!found.iter().any(|(f, _)| *f == skipped), "{skipped}: {found:?}");
    }
    let limits = report.coverage.limits.join("\n");
    assert!(
        limits.contains("Rare: the 100 most frequent of 150 values in the subset"),
        "{limits}"
    );
    // Only the strongest are shown when the request caps them.
    let one = error_outliers(&store, &OutlierRequest { limit: 1, ..req }).unwrap();
    assert_eq!(one.result.outliers.len(), 1);
    assert!(
        one.coverage
            .limits
            .iter()
            .any(|l| l.contains("outliers found; the 1 strongest are shown")),
        "{:?}",
        one.coverage.limits
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_window_is_a_filter_the_subset_narrows_and_the_candidate_fields_are_capped() {
    // 70 fields with two values each, errors on every fifth event.
    let mut text = String::new();
    for i in 0..60 {
        let fields: Vec<String> = (0..70)
            .map(|f| format!("\"F{f:02}\":\"{}\"", if (i % 5 == 0) == (f % 2 == 0) { "a" } else { "b" }))
            .collect();
        text.push_str(&line(i, if i % 5 == 0 { "ERR" } else { "INF" }, &fields.join(",")));
    }
    let (store, dir) = store_of(&text);
    let req = OutlierRequest {
        query: Filter {
            ql: "@level >= verbose".into(),
            ..Filter::default()
        },
        ..OutlierRequest::default()
    };
    let report = error_outliers(&store, &req).unwrap();
    assert_eq!((report.result.subset_total, report.result.rest_total), (12, 48));
    assert_eq!(report.result.fields_checked, imt_core::analysis::outliers::MAX_FIELDS);
    assert!(
        report
            .coverage
            .limits
            .iter()
            .any(|l| l.contains("the 60 most populated were examined")),
        "{:?}",
        report.coverage.limits
    );
    assert!(!report.result.outliers.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_outlier_whose_evidence_cannot_be_listed_is_still_reported() {
    let (store, dir) = store();
    // The two counts of the window and the subset are answered; the pages that list the evidence fail.
    let wrapped = Delegating::failing_pages_from(&store, 2);
    let report = error_outliers(&wrapped, &OutlierRequest::default()).unwrap();
    assert!(!report.result.outliers.is_empty());
    assert!(report.result.outliers.iter().all(|o| o.sample_ids.is_empty()));
    let _ = std::fs::remove_dir_all(dir);
}
