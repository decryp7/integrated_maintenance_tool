//! The query language on the DuckDB store answers like the in-memory `Workspace` on `/samples`
//! (events by source path and line, tables as they are), and is cancellable.

mod common;

use common::{pair, ranged, Pair};
use imt_core::api::{CompletionRequest, QueryRequest};
use imt_core::ports::{Filter, Query, StoreError};
use imt_core::ql::{QlResult, QueryControl};
use imt_core::Profile;
use std::collections::HashMap;
use std::ops::ControlFlow;

/// A result with floats rounded to 10 significant digits (SQL sums in another order than the engine).
fn rounded(r: QlResult) -> QlResult {
    let round = |v: serde_json::Value| match v.as_f64() {
        Some(f) if v.is_f64() => format!("{f:.9e}")
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or(v, serde_json::Value::Number),
        _ => v,
    };
    match r {
        QlResult::Table { columns, rows } => QlResult::Table {
            columns,
            rows: rows.into_iter().map(|row| row.into_iter().map(round).collect()).collect(),
        },
        other => other,
    }
}

fn keys(p: &Pair) -> (HashMap<u32, String>, HashMap<u64, String>) {
    let ws =
        p.ws.records()
            .iter()
            .map(|r| (r.id, format!("{}:{}", p.ws_paths[&r.source], r.line)))
            .collect();
    let mut duck = HashMap::new();
    p.duck
        .scan(&Filter::default(), &mut |b| {
            for r in b.rows() {
                duck.insert(r.id().0, format!("{}:{}", p.duck_paths[&r.source().0], r.line()));
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    (ws, duck)
}

#[test]
fn programs_run_like_the_workspace() {
    let p = pair(Profile::default_profile());
    let (ws_keys, duck_keys) = keys(&p);
    let programs = [
        "level >= warning",
        "level >= warning | stats count() as n by source | sort n desc",
        "| stats count(), avg(ElapsedMs) as avg, p95(ElapsedMs), count_distinct(MachineName) by level",
        "ElapsedMs > 100 | sort ElapsedMs desc | head 25",
        "message contains order | where MachineName = WEB01 | fields MachineName, ElapsedMs",
        "Path ~ 'orders$' | stats max(ElapsedMs) as hi, min(ElapsedMs), sum(ElapsedMs) by MachineName | where hi > 0",
        "| sort @line | head 10 | where level = information",
        "timeout or \"payment failed\" | stats count() by MachineName, level | sort MachineName, level",
    ];
    for ql in programs {
        let req = QueryRequest {
            query: ranged(
                &p,
                Filter {
                    ql: ql.into(),
                    ..Filter::default()
                },
            ),
        };
        let w = p.ws.run_query(&req).unwrap();
        let d = p.duck.run_query(&req).unwrap();
        match (w, d) {
            (QlResult::Events { ids: wi, fields: wf }, QlResult::Events { ids: di, fields: df }) => {
                assert_eq!(df, wf, "{ql}: fields");
                let wk: Vec<&String> = wi.iter().map(|i| &ws_keys[i]).collect();
                let dk: Vec<&String> = di.iter().map(|i| &duck_keys[&u64::from(*i)]).collect();
                assert_eq!(dk, wk, "{ql}: events");
                assert!(!wk.is_empty(), "{ql} matches events");
            }
            (w @ QlResult::Table { .. }, d) => assert_eq!(rounded(d), rounded(w), "{ql}: table"),
            (w, d) => panic!("{ql}: {w:?} vs {d:?}"),
        }
    }
    // Errors keep their span; unknown fields are found against the loaded fields.
    let bad = QueryRequest {
        query: Filter {
            ql: "level >=".into(),
            ..Filter::default()
        },
    };
    match p.duck.run_query(&bad) {
        Err(StoreError::Query(e)) => assert_eq!(Some(e), p.ws.run_query(&bad).err()),
        other => panic!("{other:?}"),
    }
    for text in ["ElapsedMss > 3", "level >= warning", "| stats count() by Machine"] {
        assert_eq!(p.duck.check_query(text).unwrap(), p.ws.check_query(text), "{text}");
    }
    let c = CompletionRequest {
        text: "Mach".into(),
        cursor: 4,
    };
    assert_eq!(p.duck.complete_query(&c).unwrap(), p.ws.complete_query(&c));
    // A cancelled run stops with Cancelled.
    let control = QueryControl::new();
    control.cancel();
    let all = QueryRequest {
        query: Filter {
            ql: "| stats count() by MachineName".into(),
            ..Filter::default()
        },
    };
    assert_eq!(p.duck.run_query_with(&all, &control), Err(StoreError::Cancelled));
    // A cancel while the statement runs interrupts it (the hook fires on another thread).
    let control = QueryControl::new();
    let c2 = control.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(5));
        c2.cancel();
    });
    let outcome = p.duck.run_query_with(&all, &control);
    canceller.join().unwrap();
    assert!(matches!(outcome, Ok(_) | Err(StoreError::Cancelled)), "{outcome:?}");
}
