//! Read benchmark (ADR 0014 budgets): every `Query` and `Analysis` method timed on a large store, with the
//! peak RSS while querying. Off by default.
//!
//! - `IMT_BENCH_READS=<n>` imports n copies of the samples (28k events each; 360 copies ≈ 10M events);
//!   `IMT_BENCH_STORE=<folder>` opens the store there (e.g. R2b's 10M-event store), or, when there is none
//!   yet, imports into it and stops (run again to measure in a process that did not import). Analyses
//!   cached by an earlier run are dropped first.
//! - `IMT_BENCH_STRICT=1` fails above the budgets (0.5 s per interactive call, 300 MB query RSS).
//!
//! Run with `--release --test bench_reads -- --nocapture`.

mod common;

// The store worker's allocator (`imt-store-worker`), so the numbers are the shipped ones.
#[global_allocator]
static GLOBAL: imt_alloc::MiMalloc = imt_alloc::MiMalloc;

use common::{log_samples, TempDir};
use imt_core::api::{QueryRequest, TimelineRequest};
use imt_core::model::Level;
use imt_core::ports::{Analysis, Diagnostics, Filter, PageQuery, Query, Search};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BUDGET_MS: u128 = 500;
const QUERY_RSS_MB: u64 = 300;

fn rss() -> u64 {
    memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn req<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).unwrap()
}

#[test]
fn read_operations_meet_the_budget() {
    imt_alloc::tune();
    let copies: Option<usize> = std::env::var("IMT_BENCH_READS").ok().and_then(|v| v.parse().ok());
    let existing = std::env::var("IMT_BENCH_STORE").ok().map(PathBuf::from);
    if copies.is_none() && existing.is_none() {
        eprintln!("skipped: set IMT_BENCH_READS=<copies> or IMT_BENCH_STORE=<folder>");
        return;
    }
    let strict = std::env::var("IMT_BENCH_STRICT").is_ok_and(|v| v == "1");
    let data = TempDir::new("reads-data");
    let dir = TempDir::new("reads");
    let store = match &existing {
        Some(folder) if folder.join(imt_store_duck::DB_FILE).is_file() => {
            // Cold analyses: results cached by an earlier run are dropped.
            if let Ok(conn) = duckdb::Connection::open(folder.join(imt_store_duck::DB_FILE)) {
                let _ = conn.execute_batch("DELETE FROM analysis_cache; CHECKPOINT;");
            }
            DuckStore::open(folder, Profile::default_profile(), PluginRegistry::new()).unwrap()
        }
        _ => {
            let paths: Vec<PathBuf> = (0..copies.unwrap_or(1).max(1))
                .map(|i| {
                    let d = data.0.join(format!("copy{i}"));
                    for p in log_samples() {
                        copy_dir(&p, &d.join(p.file_name().unwrap()));
                    }
                    d
                })
                .collect();
            let started = Instant::now();
            // With IMT_BENCH_STORE naming a new folder, the store is made there and kept: run again to
            // measure queries in a process that did not import (its RSS is the query RSS).
            let target = existing.clone().unwrap_or_else(|| dir.0.clone());
            let store = DuckStore::create(&target, Profile::default_profile(), PluginRegistry::new()).unwrap();
            let report = imt_core::ports::Ingest::open(&store, &paths).unwrap().wait().unwrap();
            eprintln!("imported {} events in {:.1} s", report.events, started.elapsed().as_secs_f64());
            let indexing = Instant::now();
            let status = store.wait_for_index(Duration::from_secs(3600));
            eprintln!("indexed in {:.1} s: {status:?}", indexing.elapsed().as_secs_f64());
            if existing.is_some() {
                eprintln!("store written to {}; run again to measure the queries", target.display());
                return;
            }
            store
        }
    };
    let summary = store.summary().unwrap();
    let (from, to) = (summary.from.unwrap_or(0), summary.to.unwrap_or(0));
    let mid = (from + to) / 2;
    let total = summary.total;
    eprintln!("{total} events, {} sources", summary.sources.len());

    let start_mb = rss() >> 20;
    let overall = Arc::new(AtomicU64::new(start_mb));
    eprintln!("RSS before querying: {start_mb} MB (includes what the import left allocated)");
    let peak = Arc::new(AtomicU64::new(rss()));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (p, s) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };
    let errors = Filter {
        levels: vec![Level::Error, Level::Fatal],
        ..Filter::default()
    };
    let text = Filter {
        text: "timeout".into(),
        ..Filter::default()
    };
    // Smart search answers from the index: wait until it caught up (a fresh import indexes in the background).
    let index = store.wait_for_index(Duration::from_secs(3600)).map_err(|e| eprintln!("{e}"));
    eprintln!("full-text index: {index:?}");
    let words = Filter {
        text: "timeout req".into(),
        text_mode: imt_core::query::TextMode::Words,
        ..Filter::default()
    };
    let window = Filter {
        from: Some(mid - 3_600_000),
        to: Some(mid + 3_600_000),
        ..Filter::default()
    };
    let page = |q: &Filter, offset: usize| PageQuery {
        query: q.clone(),
        offset,
        limit: 500,
        ..PageQuery::default()
    };
    let mut results: Vec<(&str, u128, bool, u64, bool)> = Vec::new();
    let op_peak = peak.clone();
    let mut time = |name: &'static str, interactive: bool, f: &mut dyn FnMut() -> bool| {
        // The peak RSS during this operation alone.
        op_peak.store(rss(), Ordering::Relaxed);
        let started = Instant::now();
        let ok = f();
        let ms = started.elapsed().as_millis();
        let mb = op_peak.load(Ordering::Relaxed).max(rss()) >> 20;
        overall.fetch_max(mb, Ordering::Relaxed);
        results.push((name, ms, interactive, mb, ok));
    };
    let any = Filter::default();
    time("summary (cold)", true, &mut || {
        store.summary().map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("summary (warm)", true, &mut || {
        store.summary().map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("page first", true, &mut || {
        store.page(&page(&any, 0)).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("page middle", true, &mut || {
        store.page(&page(&any, total / 2)).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("page last", true, &mut || {
        store
            .page(&page(&any, total.saturating_sub(500)))
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("page errors", true, &mut || {
        store.page(&page(&errors, 0)).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("page text", true, &mut || {
        store.page(&page(&text, 0)).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("page words (smart search)", true, &mut || {
        store.page(&page(&words, 0)).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("timeline words (smart search)", true, &mut || {
        store
            .timeline(&TimelineRequest {
                query: words.clone(),
                buckets: 400,
                always_show_lifecycle: true,
            })
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("partial matches (background scan)", false, &mut || {
        store.partial_matches(&words).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("page one hour", true, &mut || {
        store.page(&page(&window, 0)).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("timeline 400", true, &mut || {
        store
            .timeline(&TimelineRequest {
                query: any.clone(),
                buckets: 400,
                always_show_lifecycle: true,
            })
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("timeline errors", true, &mut || {
        store
            .timeline(&TimelineRequest {
                query: errors.clone(),
                buckets: 400,
                always_show_lifecycle: true,
            })
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    for field in ["@level", "@source", "MachineName"] {
        let name: &'static str = Box::leak(format!("facet {field}").into_boxed_str());
        time(name, true, &mut || {
            store
                .facet(&req(json!({"query": any, "field": field, "limit": 20})))
                .map_err(|e| eprintln!("{e}"))
                .is_ok()
        });
    }
    time("chart count by level", true, &mut || {
        store
            .chart(&req(
                json!({"query": any, "def": {"id": "c", "title": "t", "y": "count()", "splitBy": ["@level"]}, "buckets": 200}),
            ))
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("field stats ElapsedMs", true, &mut || {
        store
            .field_stats(&req(json!({"query": any, "field": "ElapsedMs", "buckets": 20})))
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("query stats", true, &mut || {
        store
            .run_query(&QueryRequest {
                query: Filter {
                    ql: "| stats count() by level".into(),
                    ..Filter::default()
                },
            })
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("check query", true, &mut || {
        store.check_query("level >= warning").map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("problems", true, &mut || store.problems().map_err(|e| eprintln!("{e}")).is_ok());
    let all = QueryRequest { query: any.clone() };
    time("patterns (cold)", false, &mut || {
        store.patterns(&all).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("patterns (cached)", true, &mut || {
        store.patterns(&all).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("exception groups", false, &mut || {
        store.exception_groups(&all).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("anomalies (cold)", false, &mut || {
        store.anomalies(&all).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("anomalies (cached)", true, &mut || {
        store.anomalies(&all).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("overview", true, &mut || store.overview(&all).map_err(|e| eprintln!("{e}")).is_ok());
    time("compare halves", false, &mut || {
        store
            .compare(&req(
                json!({"a": Filter { to: Some(mid), ..any.clone() }, "b": Filter { from: Some(mid + 1), ..any.clone() }}),
            ))
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("correlate", true, &mut || {
        store
            .correlate(&req(json!({"field": "MachineName", "value": "WEB01"})))
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("traces", false, &mut || {
        store.traces(&req(json!({"query": any}))).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("requests", false, &mut || {
        store.requests(&req(json!({"query": any}))).map_err(|e| eprintln!("{e}")).is_ok()
    });
    time("moment 1 h", true, &mut || {
        store
            .moment(&req(json!({"query": any, "from": mid - 1_800_000, "to": mid + 1_800_000})))
            .map_err(|e| eprintln!("{e}"))
            .is_ok()
    });
    time("runs", false, &mut || {
        store.runs(&req(json!({}))).map_err(|e| eprintln!("{e}")).is_ok()
    });
    stop.store(true, Ordering::Relaxed);
    let _ = sampler.join();
    let peak_mb = overall.load(Ordering::Relaxed).max(peak.load(Ordering::Relaxed) >> 20);
    eprintln!("{:<26} {:>9}  {:>7}  budget", "operation", "ms", "RSS MB");
    let mut over = Vec::new();
    let mut failed = Vec::new();
    for (name, ms, interactive, mb, ok) in &results {
        let mark = if !ok {
            failed.push(*name);
            "  FAILED"
        } else if *interactive && *ms > BUDGET_MS {
            "  OVER"
        } else {
            ""
        };
        eprintln!(
            "{name:<26} {ms:>9}  {mb:>7}{}{mark}",
            if *interactive { "  0.5 s" } else { "  batch" }
        );
        if *interactive && *ms > BUDGET_MS {
            over.push(*name);
        }
    }
    eprintln!("peak RSS while querying: {peak_mb} MB (budget {QUERY_RSS_MB} MB)");
    assert!(failed.is_empty(), "failed: {failed:?}");
    if strict {
        assert!(over.is_empty(), "over the 0.5 s budget: {over:?}");
        assert!(peak_mb <= QUERY_RSS_MB, "query RSS {peak_mb} MB");
    }
}
