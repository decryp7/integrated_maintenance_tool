//! The measured query suite and JSON-lines output.

use crate::port::{Column, Filter, SpikeStore};
use anyhow::Result;
use imt_core::model::Level;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

pub const PAGE: usize = 500;
pub const TIMELINE_BUCKETS: usize = 1000;
pub const REPEATS: usize = 3;
/// The search term: present in a few percent of the events (timeouts of the payment provider,
/// the outbox and the gateway).
pub const SEARCH: &str = "timeout";

/// Prints one measurement as a JSON line.
pub fn emit(engine: &str, op: &str, v: Value) {
    let mut o = json!({ "engine": engine, "op": op });
    if let (Some(o), Value::Object(v)) = (o.as_object_mut(), v) {
        o.extend(v);
    }
    println!("{o}");
}

/// Runs `f` [`REPEATS`] times; emits the first and best wall time and the first result.
fn measure<T: std::fmt::Debug>(engine: &str, op: &str, mut f: impl FnMut() -> Result<T>, digest: impl Fn(&T) -> String) -> Result<T> {
    let mut first = None;
    let mut times = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        let t = Instant::now();
        let r = f()?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        if first.is_none() {
            first = Some(r);
        }
    }
    let first = first.ok_or_else(|| anyhow::anyhow!("no run"))?;
    let best = times.iter().copied().fold(f64::INFINITY, f64::min);
    emit(
        engine,
        op,
        json!({ "first_ms": round(times[0]), "best_ms": round(best), "result": digest(&first) }),
    );
    Ok(first)
}

fn round(ms: f64) -> f64 {
    (ms * 10.0).round() / 10.0
}

/// The selective filter of the "filtered" pages: warnings and worse, mentioning "timeout", of one
/// instance.
pub fn selective() -> Filter {
    Filter {
        levels: vec![Level::Warning, Level::Error, Level::Fatal],
        text: Some(SEARCH.into()),
        field: Some((Column::MachineName, "WEB01-01".into())),
    }
}

pub fn suite(engine: &str, s: &dyn SpikeStore) -> Result<()> {
    let all = Filter::default();
    let page_digest = |p: &crate::port::PageOut| format!("total {} rows {} first {:?}", p.total, p.ids.len(), p.ids.first());
    let first = measure(engine, "page_first", || s.page(&all, 0, PAGE), page_digest)?;
    let total = first.total;
    measure(engine, "page_middle", || s.page(&all, total / 2, PAGE), page_digest)?;
    measure(
        engine,
        "page_last",
        || s.page(&all, total.saturating_sub(PAGE as u64), PAGE),
        page_digest,
    )?;

    let f = selective();
    let ff = measure(engine, "filtered_page_first", || s.page(&f, 0, PAGE), page_digest)?;
    let ft = ff.total;
    measure(engine, "filtered_page_middle", || s.page(&f, ft / 2, PAGE), page_digest)?;
    measure(
        engine,
        "filtered_page_last",
        || s.page(&f, ft.saturating_sub(PAGE as u64), PAGE),
        page_digest,
    )?;

    let facet_digest = |x: &crate::port::FacetOut| format!("total {} distinct {} top {:?}", x.total, x.distinct, x.top.first());
    measure(engine, "facet_machine", || s.facet(Column::MachineName, &all, 50), facet_digest)?;
    let problems = Filter {
        levels: vec![Level::Warning, Level::Error, Level::Fatal],
        ..Filter::default()
    };
    measure(
        engine,
        "facet_context_filtered",
        || s.facet(Column::SourceContext, &problems, 50),
        facet_digest,
    )?;

    let tl_digest = |t: &crate::port::TimelineOut| format!("events {} lanes {} buckets {}", t.total(), t.lanes, t.histogram.len());
    measure(engine, "timeline_1000", || s.timeline(&all, TIMELINE_BUCKETS), tl_digest)?;
    let errors = Filter {
        levels: vec![Level::Error, Level::Fatal],
        ..Filter::default()
    };
    measure(engine, "timeline_1000_errors", || s.timeline(&errors, TIMELINE_BUCKETS), tl_digest)?;

    measure(
        engine,
        "field_stats_elapsed",
        || s.field_stats(Column::Elapsed, &all),
        |x| {
            format!(
                "count {} distinct {} mean {:.2} p95 {:.1} max {:.1}",
                x.count, x.distinct, x.mean, x.p95, x.max
            )
        },
    )?;
    measure(
        engine,
        "search_timeout",
        || s.search(SEARCH, PAGE),
        |x| format!("hits {} fetched {}", x.hits, x.fetched),
    )?;
    Ok(())
}

/// Bytes under `dir`.
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.filter_map(|e| e.ok())
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}
