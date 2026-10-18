//! Persistent-store spike (docs/store-spike.md, docs/adr/0010-persistent-store.md). Not shipped.
//!
//! ```text
//! imt-store-spike gen <corpus-dir> <events>           write the scaled CLEF corpus
//! imt-store-spike mem <corpus-dir> [copies]           in-memory Workspace: ingest + suite
//! imt-store-spike ingest duck|fusion <corpus> <store> [copies]
//! imt-store-spike query duck|fusion <store>           reopen + suite + SQL console checks
//! imt-store-spike console duck|fusion <store>         reopen + SQL console checks only
//! ```
//! Output is one JSON object per line. `SPIKE_MAX_RSS_GB` (default 10) aborts a run that grows
//! past it.

mod bench;
mod corpus;
#[cfg(feature = "duck")]
mod duck;
mod fts;
#[cfg(feature = "fusion")]
mod fusion;
mod inmem;
mod port;
mod rows;
mod rss;

use anyhow::{bail, Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// About half a million events per ingest chunk (23 copies of the samples).
const COPIES_PER_CHUNK: usize = 23;

fn samples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn arg<'a>(args: &'a [String], i: usize, usage: &str) -> Result<&'a str> {
    args.get(i)
        .map(String::as_str)
        .with_context(|| format!("usage: imt-store-spike {usage}"))
}

fn sampler() -> rss::Sampler {
    let gb: u64 = std::env::var("SPIKE_MAX_RSS_GB").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    rss::Sampler::start(gb << 30)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => {
            let usage = "gen <out-dir> <events>";
            let out = PathBuf::from(arg(&args, 1, usage)?);
            let events: u64 = arg(&args, 2, usage)?.parse()?;
            let t = Instant::now();
            let r = corpus::generate(&samples_dir(), &out, events)?;
            println!(
                "{} events, {} copies of {} events every {} ms, {} files, {} MB, in {} ms",
                r.events,
                r.copies,
                r.per_copy,
                r.period_ms,
                r.files,
                r.bytes >> 20,
                t.elapsed().as_millis()
            );
        }
        Some("stats") => {
            let samples = samples_dir();
            let mut ws = imt_core::Workspace::new(corpus::sample_profile(&samples)?)?;
            ws.open(&corpus::sample_paths(&samples)?)?;
            let s = ws.summary();
            println!("records {} from {:?} to {:?} sources {}", s.total, s.from, s.to, s.sources.len());
            for f in &s.fields {
                println!(
                    "{:<28} count {:>6} distinct {:>6} numeric {}",
                    f.name, f.count, f.distinct, f.numeric
                );
            }
        }
        Some("mem") => {
            let usage = "mem <corpus> [copies]";
            let corpus = PathBuf::from(arg(&args, 1, usage)?);
            let copies = args.get(2).map(|c| c.parse()).transpose()?;
            mem(&corpus, copies)?;
        }
        Some("ingest") => {
            let usage = "ingest duck|fusion <corpus> <store> [copies]";
            let engine = arg(&args, 1, usage)?;
            let corpus = PathBuf::from(arg(&args, 2, usage)?);
            let store = PathBuf::from(arg(&args, 3, usage)?);
            let copies = args.get(4).map(|c| c.parse()).transpose()?;
            ingest(engine, &corpus, &store, copies)?;
        }
        Some("query") => {
            let usage = "query duck|fusion <store>";
            query(arg(&args, 1, usage)?, Path::new(arg(&args, 2, usage)?), true)?;
        }
        Some("console") => {
            let usage = "console duck|fusion <store>";
            query(arg(&args, 1, usage)?, Path::new(arg(&args, 2, usage)?), false)?;
        }
        _ => bail!("usage: imt-store-spike gen|stats|mem|ingest|query ... (see the crate docs)"),
    }
    Ok(())
}

fn mem(corpus: &Path, copies: Option<usize>) -> Result<()> {
    let s = sampler();
    let dirs = corpus::copy_dirs(corpus, copies)?;
    let mut ws = imt_core::Workspace::new(corpus::sample_profile(&samples_dir())?)?;
    let t = Instant::now();
    ws.open(&dirs)?;
    let ms = t.elapsed().as_millis();
    let events = ws.records().len();
    bench::emit(
        "mem",
        "ingest",
        json!({ "events": events, "copies": dirs.len(), "ms": ms, "events_per_s": events as f64 / t.elapsed().as_secs_f64(),
                "peak_rss_mb": s.take_peak_mb(), "rss_after_mb": rss::current() >> 20 }),
    );
    let store = inmem::InMemory { store: ws };
    bench::suite("mem", &store)?;
    bench::emit("mem", "query_phase", json!({ "peak_rss_mb": s.take_peak_mb() }));
    Ok(())
}

/// The engine-specific writer behind one interface for the ingest loop.
trait Ingest {
    fn append(&mut self, chunk: corpus::Chunk<'_>) -> Result<()>;
    fn finish(self: Box<Self>, started: Instant) -> Result<fts::Finished>;
}

#[cfg(feature = "duck")]
impl Ingest for duck::DuckIngest {
    fn append(&mut self, chunk: corpus::Chunk<'_>) -> Result<()> {
        duck::DuckIngest::append(self, chunk)
    }
    fn finish(self: Box<Self>, started: Instant) -> Result<fts::Finished> {
        duck::DuckIngest::finish(*self, started)
    }
}

#[cfg(feature = "fusion")]
impl Ingest for fusion::FusionIngest {
    fn append(&mut self, chunk: corpus::Chunk<'_>) -> Result<()> {
        fusion::FusionIngest::append(self, chunk)
    }
    fn finish(self: Box<Self>, started: Instant) -> Result<fts::Finished> {
        fusion::FusionIngest::finish(*self, started)
    }
}

fn writer(engine: &str, store: &Path) -> Result<Box<dyn Ingest>> {
    match engine {
        #[cfg(feature = "duck")]
        "duck" => Ok(Box::new(duck::DuckIngest::create(store)?)),
        #[cfg(feature = "fusion")]
        "fusion" => Ok(Box::new(fusion::FusionIngest::create(store)?)),
        other => bail!("unknown or disabled engine `{other}`"),
    }
}

fn ingest(engine: &str, corpus: &Path, store: &Path, copies: Option<usize>) -> Result<()> {
    let s = sampler();
    let dirs = corpus::copy_dirs(corpus, copies)?;
    let profile = corpus::sample_profile(&samples_dir())?;
    let mut w = writer(engine, store)?;
    let t = Instant::now();
    let mut write = Duration::ZERO;
    let (events, sources) = corpus::for_each_chunk(&dirs, &profile, COPIES_PER_CHUNK, |chunk| {
        let tw = Instant::now();
        w.append(chunk)?;
        write += tw.elapsed();
        Ok(())
    })?;
    let parse_ms = t.elapsed().as_millis() - write.as_millis();
    let fin = w.finish(t)?;
    let fts = bench::dir_size(&store.join("fts"));
    let all = bench::dir_size(store);
    bench::emit(
        engine,
        "ingest",
        json!({ "events": events, "sources": sources, "copies": dirs.len(),
                "parse_ms": parse_ms, "write_ms": write.as_millis(), "table_ms": fin.table_ms, "total_ms": fin.total_ms,
                "indexed_at_table": fin.indexed_at_table, "indexed": fin.indexed, "cancelled": fin.cancelled,
                "events_per_s": events as f64 / (fin.total_ms.max(1) as f64 / 1000.0),
                "table_mb": (all - fts) >> 20, "fts_mb": fts >> 20, "peak_rss_mb": s.take_peak_mb() }),
    );
    Ok(())
}

fn query(engine: &str, store: &Path, suite: bool) -> Result<()> {
    let s = sampler();
    let t = Instant::now();
    match engine {
        #[cfg(feature = "duck")]
        "duck" => {
            let db = duck::DuckStore::open(store)?;
            reopened(engine, t, db.total, db.fts(), &s)?;
            if suite {
                bench::suite(engine, &db)?;
                bench::emit(engine, "query_phase", json!({ "peak_rss_mb": s.take_peak_mb() }));
            }
            console_checks(engine, |sql, timeout| db.console(sql, timeout, 1000).map(|r| r.len()))?;
        }
        #[cfg(feature = "fusion")]
        "fusion" => {
            let db = fusion::FusionStore::open(store)?;
            reopened(engine, t, db.total, db.fts(), &s)?;
            if suite {
                bench::suite(engine, &db)?;
                bench::emit(engine, "query_phase", json!({ "peak_rss_mb": s.take_peak_mb() }));
            }
            console_checks(engine, |sql, timeout| {
                db.console(sql, timeout, 1000).map(|b| b.iter().map(|b| b.num_rows()).sum())
            })?;
        }
        other => bail!("unknown or disabled engine `{other}`"),
    }
    Ok(())
}

fn reopened(engine: &str, t: Instant, total: u64, fts: &fts::Fts, s: &rss::Sampler) -> Result<()> {
    let ms = t.elapsed().as_millis();
    bench::emit(
        engine,
        "reopen",
        json!({ "ms": ms, "events": total, "indexed": fts.docs(), "indexed_to": fts.indexed_to()?,
                "rss_mb": rss::current() >> 20, "peak_rss_mb": s.take_peak_mb() }),
    );
    Ok(())
}

/// The SQL console's guards: reads work and are capped, writes and file access fail, a runaway
/// query is stopped by its timeout.
fn console_checks(engine: &str, run: impl Fn(&str, Duration) -> Result<usize>) -> Result<()> {
    let checks = [
        ("select", "SELECT level, count(*) FROM events GROUP BY level ORDER BY level", 10u64),
        ("row_cap", "SELECT id FROM events", 10),
        ("write", "DELETE FROM events WHERE id < 10", 10),
        ("ddl", "CREATE TABLE x AS SELECT 1", 10),
        ("file_read", "SELECT * FROM read_csv('C:/Windows/win.ini')", 10),
        ("file_write", "COPY (SELECT 1) TO 'spike-escape.csv'", 10),
        ("timeout", "SELECT count(*) FROM events a, events b WHERE a.message < b.message", 2),
        // Is the cancelled query really gone, or still holding the engine's threads?
        (
            "after_timeout",
            "SELECT level, count(*) FROM events GROUP BY level ORDER BY level",
            10,
        ),
    ];
    for (name, sql, timeout_s) in checks {
        let t = Instant::now();
        let r = run(sql, Duration::from_secs(timeout_s));
        let outcome = match r {
            Ok(rows) => format!("ok, {rows} rows"),
            Err(e) => format!("refused: {}", e.to_string().lines().next().unwrap_or_default()),
        };
        bench::emit(
            engine,
            &format!("console_{name}"),
            json!({ "ms": t.elapsed().as_millis(), "outcome": outcome }),
        );
    }
    let _ = std::fs::remove_file("spike-escape.csv");
    Ok(())
}
