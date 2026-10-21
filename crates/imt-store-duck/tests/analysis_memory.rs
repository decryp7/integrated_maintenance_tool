//! Memory budget of the analyses (ADR 0014: 300 MB resident while querying): anomaly detection, the Moment and
//! compare run on a store of a generated corpus while a sampler thread polls the process RSS every 20 ms. Each
//! may add little to what the process held before it: per-instance aggregates and DuckDB's working memory (within a
//! lowered limit), never memory per event (anomaly detection kept a few dozen bytes per event).
//!
//! `IMT_BENCH_EVENTS=<n>` sets the corpus size (default 1M, where the in-memory detection added 250 MB); `IMT_BENCH_STORE=<folder>` measures an
//! existing store instead (e.g. `bench_reads`' 10M events; its cached analyses are dropped first). Measure with
//! `--release`.

mod common;

// The store worker's allocator and its tuning, so the numbers are the shipped ones.
#[global_allocator]
static GLOBAL: imt_alloc::MiMalloc = imt_alloc::MiMalloc;

use common::rss::{current as rss, Sampler};
use common::TempDir;
use imt_core::api::QueryRequest;
use imt_core::ports::{Analysis, Filter, Query};
use imt_core::Profile;
use serde_json::json;
use std::time::Instant;

/// The worker's budget while querying (ADR 0014).
const QUERY_BUDGET_MB: u64 = 300;

/// DuckDB's memory limit while analysing, below the shipped one so that memory held per event on the Rust side
/// would stand out (at 1M events the in-memory anomaly detection added 250 MB).
const DUCK_LIMIT_MB: u64 = 64;

/// Most an analysis may add to the RSS before it: DuckDB's sorts and windows within its memory limit, then the
/// aggregates and the result.
const GROWTH_MB: u64 = DUCK_LIMIT_MB + 50;

fn req<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).unwrap()
}

#[test]
fn analyses_stay_within_the_query_budget() {
    imt_alloc::tune();
    let events: u64 = std::env::var("IMT_BENCH_EVENTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let base = TempDir::new("analysis-memory-corpus");
    let dir = TempDir::new("analysis-memory");
    let store = match std::env::var("IMT_BENCH_STORE").ok().map(std::path::PathBuf::from) {
        // An existing store (e.g. `bench_reads`' 10M events), opened in this process: cold analyses.
        Some(folder) => {
            if let Ok(conn) = duckdb::Connection::open(folder.join(imt_store_duck::DB_FILE)) {
                let _ = conn.execute_batch("DELETE FROM analysis_cache; CHECKPOINT;");
            }
            imt_store_duck::DuckStore::open(&folder, Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap()
        }
        None => {
            let corpus = common::corpus::corpus(&base.0.join("corpus"), events);
            let (store, report) = common::store_with(&dir, Profile::default_profile(), &[corpus]).unwrap();
            assert_eq!(store.summary().unwrap().total, report.events);
            store
        }
    };
    let summary = store.summary().unwrap();
    store
        .configure(imt_store_duck::StoreOptions {
            memory_limit: format!("{DUCK_LIMIT_MB}MB"),
            ..imt_store_duck::StoreOptions::default()
        })
        .unwrap();
    let (from, to) = (summary.from.unwrap(), summary.to.unwrap());
    let mid = (from + to) / 2;
    let all = QueryRequest { query: Filter::default() };
    let sampler = Sampler::start();
    let mut over = Vec::new();
    let mut measure = |name: &str, op: &mut dyn FnMut() -> usize| {
        let before = rss() >> 20;
        sampler.take_peak_mb();
        let started = Instant::now();
        let found = op();
        let peak = sampler.take_peak_mb();
        eprintln!(
            "{name}: {found} found in {:.2} s; RSS before {before} MB, peak {peak} MB (+{} MB)",
            started.elapsed().as_secs_f64(),
            peak.saturating_sub(before)
        );
        if peak > QUERY_BUDGET_MB || peak.saturating_sub(before) > GROWTH_MB {
            over.push(format!("{name}: peak {peak} MB, +{} MB", peak.saturating_sub(before)));
        }
    };
    eprintln!("{} events in {} sources", summary.total, summary.sources.len());
    measure("anomalies", &mut || {
        let found = store.anomalies(&all).unwrap();
        assert_eq!(found.coverage.analysed, summary.total as u64, "every event is analysed");
        found.result.len()
    });
    measure("moment", &mut || {
        let m = store
            .moment(&req(
                json!({"query": Filter::default(), "from": mid - 1_800_000, "to": mid + 1_800_000}),
            ))
            .unwrap();
        m.result.anomalies.len()
    });
    measure("compare", &mut || {
        let c = store
            .compare(&req(
                json!({"a": Filter { to: Some(mid), ..Filter::default() }, "b": Filter { from: Some(mid + 1), ..Filter::default() }}),
            ))
            .unwrap();
        c.result.changed_fields.len()
    });
    assert!(over.is_empty(), "over the query budget: {over:?}");
}
