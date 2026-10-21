//! Differential tests of the analyses: the DuckDB store (events streamed from the database) answers
//! like the in-memory `Workspace` on `/samples`. Event ids differ between the stores, so results are
//! compared with every event id replaced by its source path and line.

mod common;

use common::{pair, ranged, sample_profile, Pair};
use imt_core::api::QueryRequest;
use imt_core::ports::{Analysis, Filter, Query};
use imt_core::Profile;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ops::ControlFlow;

/// Event id -> "path:line", per store.
struct Ids {
    ws: HashMap<u64, String>,
    duck: HashMap<u64, String>,
}

fn ids(p: &Pair) -> Ids {
    let ws =
        p.ws.records()
            .iter()
            .map(|r| (u64::from(r.id), format!("{}:{}", p.ws_paths[&r.source], r.line)))
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
    Ids { ws, duck }
}

/// `v` with event ids replaced by "path:line" (fields that hold event ids: `sampleIds`, `firstError`,
/// `recordId`, and `id` of objects with a `ts`).
fn normalize(v: &Value, map: &HashMap<u64, String>) -> Value {
    let id = |x: &Value| match x.as_u64() {
        Some(n) => Value::String(map.get(&n).cloned().unwrap_or_else(|| format!("unknown {n}"))),
        None => x.clone(),
    };
    match v {
        Value::Array(a) => Value::Array(a.iter().map(|x| normalize(x, map)).collect()),
        Value::Object(o) => {
            let has_ts = o.contains_key("ts");
            Value::Object(
                o.iter()
                    .map(|(k, x)| {
                        let x = match k.as_str() {
                            "sampleIds" => Value::Array(x.as_array().into_iter().flatten().map(id).collect()),
                            "firstError" | "recordId" => id(x),
                            "id" if has_ts => id(x),
                            _ => normalize(x, map),
                        };
                        (k.clone(), x)
                    })
                    .collect(),
            )
        }
        // Sums in SQL add in another order than the engine: compare floats to 10 significant digits.
        Value::Number(n) if n.is_f64() => n
            .as_f64()
            .and_then(|f| format!("{f:.9e}").parse::<f64>().ok())
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| v.clone(), Value::Number),
        _ => v.clone(),
    }
}

fn req<T: DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn same(name: &str, ws: Value, duck: Value, ids: &Ids) {
    let (w, d) = (normalize(&ws, &ids.ws), normalize(&duck, &ids.duck));
    if w != d {
        let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap();
        let (w, d) = (pretty(&w), pretty(&d));
        let line = w.lines().zip(d.lines()).position(|(a, b)| a != b).unwrap_or(0);
        panic!(
            "{name}: the stores differ at line {line}\n--- workspace ---\n{}\n--- duckdb ---\n{}",
            w.lines().skip(line.saturating_sub(5)).take(15).collect::<Vec<_>>().join("\n"),
            d.lines().skip(line.saturating_sub(5)).take(15).collect::<Vec<_>>().join("\n")
        );
    }
}

fn to<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap()
}

fn check_analyses(p: &Pair) {
    let ids = ids(p);
    let all = ranged(p, Filter::default());
    let mid = (p.range.0 + p.range.1) / 2;
    let queries = [
        ("all", all.clone()),
        (
            "warnings and errors",
            ranged(
                p,
                Filter {
                    ql: "level >= warning".into(),
                    ..Filter::default()
                },
            ),
        ),
        (
            "regex (scan)",
            ranged(
                p,
                Filter {
                    text: "order|payment".into(),
                    regex: true,
                    ..Filter::default()
                },
            ),
        ),
    ];
    let mut covered = 0;
    for (name, q) in &queries {
        let r = QueryRequest { query: q.clone() };
        let d = p.duck.patterns(&r).unwrap();
        assert!(d.coverage.complete(), "{name}: {:?}", d.coverage);
        assert!(!d.result.is_empty(), "{name}: patterns");
        covered += d.coverage.analysed;
        same(&format!("patterns {name}"), to(p.ws.patterns(&r).unwrap()), to(&d.result), &ids);
        // A second request is served from the cache.
        assert_eq!(p.duck.patterns(&r).unwrap(), d);
        same(
            &format!("exceptions {name}"),
            to(p.ws.exception_groups(&r).unwrap()),
            to(p.duck.exception_groups(&r).unwrap().result),
            &ids,
        );
        same(
            &format!("anomalies {name}"),
            to(p.ws.anomalies(&r).unwrap()),
            to(p.duck.anomalies(&r).unwrap().result),
            &ids,
        );
        same(
            &format!("overview {name}"),
            to(p.ws.overview(&r).unwrap()),
            to(p.duck.overview(&r).unwrap().result),
            &ids,
        );
        same(
            &format!("traces {name}"),
            to(p.ws.traces(&req(json!({"query": q}))).unwrap()),
            to(p.duck.traces(&req(json!({"query": q}))).unwrap().result),
            &ids,
        );
        for field in ["ElapsedMs", "MachineName", "@level", "@source", "CpuPercent"] {
            let fr = req(json!({"query": q, "field": field, "buckets": 20}));
            same(
                &format!("field stats {field} {name}"),
                to(p.ws.field_stats(&fr).unwrap()),
                to(p.duck.field_stats(&fr).unwrap()),
                &ids,
            );
        }
        let rr = req(json!({"query": q, "idField": "RequestId|CorrelationId"}));
        same(
            &format!("requests {name}"),
            to(p.ws.requests(&rr).unwrap()),
            to(p.duck.requests(&rr).unwrap().result),
            &ids,
        );
        for def in [
            json!({"id": "c", "title": "Events", "y": "count()", "splitBy": ["@level"]}),
            json!({"id": "d", "title": "Elapsed", "y": "avg(ElapsedMs)", "splitBy": ["MachineName"], "bucket": "1m"}),
            json!({"id": "e", "title": "Slowest", "y": "max(ElapsedMs)", "splitBy": ["MachineName"]}),
            json!({"id": "f", "title": "Fastest", "y": "min(ElapsedMs)"}),
            json!({"id": "g", "title": "Latest", "y": "last(ElapsedMs)", "bucket": "5m"}),
        ] {
            let cr = req(json!({"query": q, "def": def, "buckets": 100}));
            same(
                &format!("chart {name}"),
                to(p.ws.chart(&cr).unwrap()),
                to(p.duck.chart(&cr).unwrap()),
                &ids,
            );
        }
    }
    assert!(covered > 10_000);
    let r = QueryRequest { query: all.clone() };
    let found = (
        p.duck.exception_groups(&r).unwrap().result.len(),
        p.duck.anomalies(&r).unwrap().result.len(),
        p.duck.overview(&r).unwrap().result.len(),
    );
    assert!(found.0 > 0 && found.1 > 0 && found.2 > 0, "{found:?}");
    // Runs between lifecycle markers (the default profile's span rules), and a filtered page of them.
    for rr in [json!({}), json!({"from": mid, "sort": "duration", "descending": true, "limit": 5})] {
        let rr: imt_core::spans::RunsRequest = req(rr);
        same("runs", to(p.ws.runs(&rr)), to(p.duck.runs(&rr).unwrap()), &ids);
    }
    assert!(p.duck.runs(&req(json!({}))).unwrap().total > 0, "the samples have runs");
    // A trace's waterfall.
    let traces = p.ws.traces(&req(json!({"query": all}))).unwrap().0;
    if let Some(t) = traces.first() {
        let tr = req(json!({"query": all, "traceId": t.trace_id}));
        same(
            "waterfall",
            to(p.ws.traces(&tr).unwrap()),
            to(p.duck.traces(&tr).unwrap().result),
            &ids,
        );
    }
    // Compare two halves of the samples.
    let cr = req(json!({
        "a": Filter { to: Some(mid), ..all.clone() },
        "b": Filter { from: Some(mid + 1), ..all.clone() },
    }));
    let d = p.duck.compare(&cr).unwrap();
    assert!(d.coverage.complete());
    same("compare", to(p.ws.compare(&cr).unwrap()), to(d.result), &ids);
    // The moment around the middle.
    let mr = req(json!({"query": Filter::default(), "from": mid - 1_800_000, "to": mid + 1_800_000}));
    same(
        "moment",
        to(p.ws.moment(&mr).unwrap()),
        to(p.duck.moment(&mr).unwrap().result),
        &ids,
    );
    // Correlation across all events.
    let host = p
        .duck
        .facet(&req(json!({"query": all, "field": "MachineName", "limit": 1})))
        .unwrap();
    if let Some(v) = host.values.first() {
        let cr = req(json!({"field": "MachineName", "value": v.value}));
        let w: Vec<String> = p.ws.correlate(&cr).iter().map(|i| ids.ws[&u64::from(*i)].clone()).collect();
        let d = p.duck.correlate(&cr).unwrap();
        let d: Vec<String> = d.result.iter().map(|i| ids.duck[&i.0].clone()).collect();
        assert_eq!(d, w, "correlate");
        assert!(!w.is_empty());
    }
}

#[test]
fn analyses_match_the_workspace_with_the_default_profile() {
    check_analyses(&pair(Profile::default_profile()));
}

#[test]
fn analyses_match_the_workspace_with_the_sample_profile() {
    let p = pair(sample_profile());
    check_analyses(&p);
    // The profile's own charts (with `when` conditions).
    let summary = p.ws.summary();
    let ids = ids(&p);
    for c in imt_core::chart::chart_defs(&p.ws.profile().clone()) {
        let cr = req(json!({"query": ranged(&p, Filter::default()), "chartId": c.id, "buckets": 50}));
        same(
            &format!("profile chart {}", c.id),
            to(p.ws.chart(&cr).unwrap()),
            to(p.duck.chart(&cr).unwrap()),
            &ids,
        );
    }
    assert!(summary.total > 0);
}
