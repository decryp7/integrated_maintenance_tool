//! The ADR 0014 benchmark at scale: `IMT_BENCH_EVENTS=10000000 cargo test --release --test bench -- --nocapture`
//! generates the corpus (once, in `IMT_BENCH_DIR`, default the temp folder), imports it, reopens the store
//! and times the interactive operations, with the peak RSS of each phase. Without `IMT_BENCH_EVENTS` it
//! does nothing.

mod common;

use common::rss::{current, Sampler};
use common::TempDir;
use imt_core::api::{FacetRequest, TimelineRequest};
use imt_core::model::Level;
use imt_core::ports::{Ingest, PageQuery, Positions, Query, Search, SearchRequest};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Interactive budget (ADR 0014): every operation under half a second, cold.
const INTERACTIVE_MS: f64 = 500.0;

fn ms<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_secs_f64() * 1000.0)
}

fn dir_mb(p: &std::path::Path) -> f64 {
    fn size(p: &std::path::Path) -> u64 {
        std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .map(|e| match e.metadata() {
                        Ok(m) if m.is_dir() => size(&e.path()),
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    size(p) as f64 / 1e6
}

#[test]
fn interactive_operations_at_scale() {
    let Some(events) = std::env::var("IMT_BENCH_EVENTS").ok().and_then(|v| v.parse::<u64>().ok()) else {
        eprintln!("skipped: set IMT_BENCH_EVENTS (e.g. 10000000) to run the benchmark");
        return;
    };
    let base = std::env::var("IMT_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let t = Instant::now();
    let corpus = common::corpus::corpus(&base.join(format!("imt-corpus-{events}")), events);
    eprintln!("corpus: {:.0} MB in {:.1} s", dir_mb(&corpus), t.elapsed().as_secs_f64());
    // `IMT_BENCH_STORE` keeps the store there and reuses it on the next run (queries only).
    let kept = std::env::var("IMT_BENCH_STORE").ok().map(PathBuf::from);
    let temp = kept.is_none().then(|| TempDir::new("bench-store"));
    let store_dir = common::TempDir(
        kept.clone()
            .unwrap_or_else(|| temp.as_ref().map(|t| t.0.clone()).unwrap_or_default()),
    );
    let reuse = kept.is_some() && store_dir.0.join("data.duckdb").is_file();
    let sampler = Sampler::start();

    // Import, then wait for the full-text index.
    if !reuse {
        let ((r, import_peak, status, index_ms, index_peak), all_ms) = ms(|| {
            let store = DuckStore::create(&store_dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
            let r = store.open(std::slice::from_ref(&corpus)).unwrap().wait().unwrap();
            let import_peak = sampler.take_peak_mb();
            let (status, index_ms) = ms(|| store.wait_for_index(Duration::from_secs(3600)).unwrap());
            (r, import_peak, status, index_ms, sampler.take_peak_mb())
        });
        let import_ms = all_ms - index_ms;
        eprintln!(
        "import: {} events in {:.1} s = {:.0} events/s, peak RSS {import_peak} MB; index: {} docs in {:.1} s = {:.0} docs/s, peak RSS {index_peak} MB; store {:.0} MB (segments {:.0} MB, index {:.0} MB)",
        r.events,
        import_ms / 1000.0,
        r.events as f64 / (import_ms / 1000.0),
        status.indexed,
        index_ms / 1000.0,
        status.indexed as f64 / (index_ms / 1000.0),
        dir_mb(&store_dir.0),
        dir_mb(&store_dir.0.join("segments")),
        dir_mb(&store_dir.0.join("index")),
    );
    }
    // Reopened: DuckDB's buffers are empty (the OS file cache is not).
    let before = current() >> 20;
    let (store, open_ms) = ms(|| DuckStore::open(&store_dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap());
    sampler.take_peak_mb();
    let idle = current() >> 20;
    eprintln!("reopen {open_ms:.0} ms; RSS {before} MB before, {idle} MB idle");
    let total = store.positioned().unwrap();
    if let Ok(file) = std::env::var("IMT_BENCH_SQL") {
        for sql in std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with("--"))
        {
            let (n, cold) = ms(|| store.bench_sql(sql));
            let (_, warm) = ms(|| store.bench_sql(sql));
            eprintln!("  {cold:.0} / {warm:.0} ms: {sql} -> {n:?}");
        }
        if kept.is_some() {
            std::mem::forget(store_dir);
        }
        return;
    }
    let mut results: Vec<(String, f64, f64)> = Vec::new();
    let mut query_peak = 0;
    let mut time = |name: &str, f: &mut dyn FnMut() -> u64| {
        sampler.take_peak_mb();
        let (n, cold) = ms(&mut *f);
        let (_, warm) = ms(&mut *f);
        let peak = sampler.take_peak_mb();
        query_peak = query_peak.max(peak);
        eprintln!("  {name}: {cold:.0} ms cold, {warm:.0} ms warm ({n}), peak RSS {peak} MB");
        results.push((name.to_string(), cold, warm));
    };
    time("summary", &mut || store.summary().unwrap().total as u64);
    for (name, at) in [("first", 0), ("middle", total / 2), ("last", total.saturating_sub(500))] {
        time(&format!("page {name} by position (ids + rows)"), &mut || {
            let ids = store.ids_at(at, 500).unwrap();
            store.rows(&ids).unwrap().iter().map(|b| b.num_rows() as u64).sum()
        });
    }
    time("page first, ORDER BY (R2a)", &mut || {
        store
            .page(&PageQuery {
                limit: 500,
                ..Default::default()
            })
            .unwrap()
            .rows
            .len() as u64
    });
    let errors = imt_core::query::Query {
        levels: vec![Level::Error, Level::Fatal],
        ..Default::default()
    };
    time("errors page with total", &mut || {
        store
            .page(&PageQuery {
                query: errors.clone(),
                limit: 500,
                ..Default::default()
            })
            .unwrap()
            .total as u64
    });
    time("timeline 1000 buckets", &mut || {
        store
            .timeline(&TimelineRequest {
                query: Default::default(),
                buckets: 1000,
                always_show_lifecycle: true,
            })
            .unwrap()
            .histogram
            .len() as u64
    });
    time("facet MachineName", &mut || {
        store
            .facet(&FacetRequest {
                query: Default::default(),
                field: "MachineName".into(),
                limit: 50,
            })
            .unwrap()
            .values
            .len() as u64
    });
    time("field stats Elapsed (SQL)", &mut || {
        store
            .bench_sql("SELECT count(x), min(x), max(x), avg(x), quantile_cont(x, [0.5, 0.95, 0.99]) FROM (SELECT TRY_CAST(extra['Elapsed'] AS DOUBLE) AS x FROM events)")
            .unwrap()
    });
    // The write-side catalog (segment fields, histograms) and promoted columns, as R2c will read them.
    for (name, sql) in [
        (
            "field list from the catalog",
            "SELECT key, sum(rows), sum(numeric_rows), max(distinct_est) FROM segment_fields GROUP BY key",
        ),
        (
            "timeline 1000 buckets from the histogram",
            "WITH h AS (SELECT * FROM histogram WHERE minute IS NOT NULL AND source IN (SELECT id FROM sources WHERE ready)), b AS (SELECT min(minute) lo, max(minute) hi FROM h) SELECT least(999, CAST(floor((minute - lo) * 1000.0 / greatest(hi - lo + 1, 1)) AS BIGINT)) k, level, sum(n) FROM h, b GROUP BY k, level",
        ),
        (
            "level and source counts from the histogram",
            "SELECT level, source, sum(n) FROM histogram WHERE source IN (SELECT id FROM sources WHERE ready) GROUP BY level, source",
        ),
        (
            "facet MachineName, promoted column",
            "SELECT \"p_MachineName\" v, count(*) n FROM events_wide WHERE v IS NOT NULL GROUP BY v ORDER BY n DESC LIMIT 50",
        ),
        (
            "field stats Elapsed, promoted column",
            "SELECT count(x), min(x), max(x), avg(x), quantile_cont(x, [0.5, 0.95, 0.99]) FROM (SELECT TRY_CAST(\"p_Elapsed\" AS DOUBLE) AS x FROM events_wide)",
        ),
        ("errors total", "SELECT count(*) FROM events WHERE level >= 4"),
        (
            "errors page ids (two-phase, first phase)",
            "SELECT id FROM events WHERE level >= 4 ORDER BY coalesce(ts, 9223372036854775807), source, line, id LIMIT 500",
        ),
    ] {
        time(name, &mut || store.bench_sql(sql).unwrap());
    }
    time("text total, ILIKE", &mut || {
        let q = imt_core::query::Query {
            text: "timeout".into(),
            ..Default::default()
        };
        store
            .page(&PageQuery {
                query: q,
                limit: 100,
                ..Default::default()
            })
            .unwrap()
            .total as u64
    });
    time("phrase total + 100 ids, Tantivy + check", &mut || {
        store
            .search(&SearchRequest {
                text: "\"timed out\"".into(),
                limit: 100,
            })
            .unwrap()
            .total
    });
    time("text total + 100 ids, Tantivy", &mut || {
        store
            .search(&SearchRequest {
                text: "timeout".into(),
                limit: 100,
            })
            .unwrap()
            .total
    });
    time("search hits fetched (100 rows)", &mut || {
        let ids = store
            .search(&SearchRequest {
                text: "timeout".into(),
                limit: 100,
            })
            .unwrap()
            .ids;
        store.rows(&ids).unwrap().iter().map(|b| b.num_rows() as u64).sum()
    });
    eprintln!("RSS peak while querying {query_peak} MB, idle after {} MB", current() >> 20);
    let slow: Vec<_> = results
        .iter()
        .filter(|(_, cold, _)| *cold > INTERACTIVE_MS)
        .map(|(n, c, _)| format!("{n} ({c:.0} ms)"))
        .collect();
    if !slow.is_empty() {
        eprintln!("over the {INTERACTIVE_MS} ms budget: {}", slow.join(", "));
    }
    if kept.is_some() {
        std::mem::forget(store_dir);
    }
}
