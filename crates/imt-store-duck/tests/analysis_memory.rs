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
/// would stand out (at 1M events the in-memory anomaly detection added 250 MB); `IMT_BENCH_DUCK_MB` overrides it.
const DUCK_LIMIT_MB: u64 = 64;

/// Most an analysis may add to the RSS before it beyond DuckDB's memory limit (DuckDB's sorts and aggregates use
/// up to that): the aggregates and the result.
const GROWTH_OVER_DUCK_MB: u64 = 50;

/// Who holds the memory, printed with each measurement on Linux (diagnosis of the Linux CI peak): the process's
/// resident memory by kind, its largest mappings, and mimalloc's statistics (IMT's Rust side; the rest of the anonymous
/// memory is DuckDB's own allocator, jemalloc in its Linux build).
fn memory_report(stage: &str) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kinds: Vec<&str> = status
        .lines()
        .filter(|l| ["VmRSS", "RssAnon", "RssFile", "VmData", "VmSize"].iter().any(|k| l.starts_with(k)))
        .collect();
    eprintln!(
        "[memory] {stage}: {}",
        kinds.join("; ").split_whitespace().collect::<Vec<_>>().join(" ")
    );
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap_or_default();
    let mut maps: Vec<(u64, String)> = Vec::new();
    let mut name = String::new();
    for line in smaps.lines() {
        if line
            .split_whitespace()
            .next()
            .is_some_and(|f| f.contains('-') && f.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        {
            name = line.split_whitespace().nth(5).unwrap_or("[anon]").to_string();
        } else if let Some(kb) = line.strip_prefix("Rss:") {
            let kb: u64 = kb.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
            maps.push((kb, name.clone()));
        }
    }
    maps.sort_by_key(|m| std::cmp::Reverse(m.0));
    let top: Vec<String> = maps.iter().take(8).map(|(kb, n)| format!("{} MB {n}", kb >> 10)).collect();
    eprintln!("[memory] {stage}: largest mappings: {}", top.join(", "));
    let mi = imt_alloc::stats_text();
    let lines: Vec<&str> = mi
        .lines()
        .filter(|l| l.contains("reserved") || l.contains("committed") || l.contains("heap stats"))
        .take(6)
        .collect();
    eprintln!("[memory] {stage}: mimalloc: {}", lines.join(" | "));
}

fn req<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).unwrap()
}

#[test]
fn analyses_stay_within_the_query_budget() {
    // DuckDB's jemalloc reads its options when libduckdb loads, before this test runs: like the store worker, the
    // measured process gets them in its environment at start. On Linux the test runs itself once more with them.
    if let Some((key, value)) = imt_store_client::allocator::worker_env(None) {
        if std::env::var(&key).ok().as_deref() != Some(value.as_str()) {
            #[allow(clippy::disallowed_methods)] // Linux only: no console window to hide.
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "analyses_stay_within_the_query_budget",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(&key, &value)
                .status()
                .unwrap();
            assert!(status.success(), "the measured run failed: {status}");
            return;
        }
    }
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
    // The full-text index is built in the background after an import (its writer's 64 MB heap and its reads
    // count against the import budget, as in the worker's budget test): measured alongside the analyses, it made
    // them look 60-70 MB larger on Linux, where indexing 1M events was still running when they started.
    let index = store.wait_for_index(std::time::Duration::from_secs(600)).unwrap();
    assert!(index.ready, "the full-text index is built: {index:?}");
    let duck_mb: u64 = std::env::var("IMT_BENCH_DUCK_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DUCK_LIMIT_MB);
    store
        .configure(imt_store_duck::StoreOptions {
            memory_limit: format!("{duck_mb}MB"),
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
        memory_report(&format!("{name} before"));
        let found = op();
        let peak = sampler.take_peak_mb();
        memory_report(&format!("{name} after"));
        eprintln!(
            "{name}: {found} found in {:.2} s; RSS before {before} MB, peak {peak} MB (+{} MB)",
            started.elapsed().as_secs_f64(),
            peak.saturating_sub(before)
        );
        if peak > QUERY_BUDGET_MB || peak.saturating_sub(before) > duck_mb + GROWTH_OVER_DUCK_MB {
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
