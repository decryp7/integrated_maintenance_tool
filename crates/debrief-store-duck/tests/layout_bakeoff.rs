//! Storage layout bake-off (ADR 0014 R2b notes): (A) a native DuckDB table written by one long-lived Arrow
//! appender versus (B) zstd Parquet segments written by the parse workers in parallel, both queried by
//! DuckDB. Ignored; run with
//! `DEBRIEF_BENCH_COPIES=20 cargo test --release --test layout_bakeoff -- --ignored --nocapture`.

mod common;

use common::{log_samples, TempDir};
use debrief_core::batch::{EventBatch, EventId, PendingBatch, SourceId};
use debrief_core::ingest::{self, BatchSink, ImportEnv, NewSource, SourceReport};
use debrief_core::Profile;
use duckdb::{Config, Connection};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Collect(Vec<EventBatch>, u64, u32);
impl BatchSink for Collect {
    fn begin_source(&mut self, _s: &NewSource) -> anyhow::Result<SourceId> {
        self.2 += 1;
        Ok(SourceId(self.2))
    }
    fn write(&mut self, b: PendingBatch) -> anyhow::Result<()> {
        let n = b.len() as u64;
        self.0.push(b.assign_ids(EventId(self.1))?);
        self.1 += n;
        Ok(())
    }
    fn finish_source(&mut self, _r: &SourceReport) -> anyhow::Result<()> {
        Ok(())
    }
    fn abort_source(&mut self, _id: SourceId) -> anyhow::Result<()> {
        Ok(())
    }
}

fn rss() -> u64 {
    memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
}

/// Peak RSS above the start of a phase.
struct Peak {
    base: u64,
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Peak {
    fn start() -> Peak {
        let base = rss();
        let peak = Arc::new(AtomicU64::new(base));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        Peak { base, peak, stop }
    }
    fn mb(self) -> i64 {
        self.stop.store(true, Ordering::Relaxed);
        (self.peak.load(Ordering::Relaxed).max(rss()) as i64 - self.base as i64) >> 20
    }
}

fn batches(copies: usize) -> Vec<EventBatch> {
    let registry = debrief_core::PluginRegistry::new();
    let parsers = registry
        .compile_parsers(&Profile::default_profile(), &debrief_core::parser::ParserSetup::builtins())
        .unwrap();
    let profile = parsers.base().clone();
    let schema = Arc::new(ingest::schema_for(&profile));
    let progress = debrief_core::workspace::ImportProgress::default();
    let env = ImportEnv {
        encodings: &Default::default(),
        importers: registry.importers(),
        parsers: &parsers,
        schema,
        cancel: progress.cancel_flag(),
    };
    let canonical: Vec<PathBuf> = log_samples().iter().map(|p| std::fs::canonicalize(p).unwrap()).collect();
    let files = ingest::files_to_read(&canonical, &canonical, &profile, &Default::default(), &Default::default());
    let mut sink = Collect(Vec::new(), 0, 0);
    for _ in 0..copies {
        ingest::import_files(&env, &files, &Default::default(), &progress, &mut sink);
    }
    sink.0
}

const DDL: &str = "CREATE TABLE events (
  id UBIGINT NOT NULL, ts BIGINT, ts_state UTINYINT NOT NULL, level UTINYINT NOT NULL, source UINTEGER NOT NULL,
  line UINTEGER NOT NULL, byte_offset UBIGINT NOT NULL, message VARCHAR NOT NULL, template VARCHAR, exception VARCHAR,
  markers UBIGINT NOT NULL, raw VARCHAR, extra MAP(VARCHAR, VARCHAR) NOT NULL);";

fn open(path: &Path) -> Connection {
    let config = Config::default()
        .max_memory("256MB")
        .unwrap()
        .with("temp_directory", path.parent().unwrap().join("tmp").to_string_lossy())
        .unwrap()
        .with("checkpoint_threshold", "1GB")
        .unwrap()
        .threads(std::env::var("DEBRIEF_DUCK_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4))
        .unwrap();
    Connection::open_with_flags(path, config).unwrap()
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    let m = e.metadata().unwrap();
                    if m.is_dir() {
                        dir_size(&e.path())
                    } else {
                        m.len()
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// `raw` set to NULL where it equals `message` (reconstructed on read with `coalesce`).
fn raw_if_different(rb: &duckdb::arrow::record_batch::RecordBatch) -> duckdb::arrow::record_batch::RecordBatch {
    use duckdb::arrow::array::{Array, StringArray};
    let msg = rb.column(7).as_any().downcast_ref::<StringArray>().unwrap();
    let raw = rb.column(11).as_any().downcast_ref::<StringArray>().unwrap();
    let out: StringArray = (0..rb.num_rows())
        .map(|i| (raw.value(i) != msg.value(i)).then(|| raw.value(i)))
        .collect();
    let mut cols = rb.columns().to_vec();
    cols[11] = Arc::new(out);
    let mut fields: Vec<_> = rb.schema().fields().iter().map(|f| f.as_ref().clone()).collect();
    fields[11] = fields[11].clone().with_nullable(true);
    duckdb::arrow::record_batch::RecordBatch::try_new(Arc::new(duckdb::arrow::datatypes::Schema::new(fields)), cols).unwrap()
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_secs_f64() * 1000.0)
}

/// The read-side queries, timed (ms), against `from` (a table or a read_parquet call).
fn queries(c: &Connection, from: &str, total: u64, facet_key: &str) {
    let order = "ORDER BY coalesce(ts, 9223372036854775807), source, line, id";
    let one = |sql: &str| -> f64 {
        let (_, ms) = time(|| {
            let mut stmt = c.prepare(sql).unwrap();
            let mut rows = stmt.query([]).unwrap();
            let mut n = 0u64;
            while let Some(_r) = rows.next().unwrap() {
                n += 1;
            }
            n
        });
        ms
    };
    let page = |offset: u64, filter: &str| {
        one(&format!(
            "WITH k AS (SELECT id FROM {from} e {filter} {order} LIMIT 500 OFFSET {offset}) SELECT id, ts, level, source, line, message, coalesce(raw, message) FROM {from} e WHERE id IN (SELECT id FROM k) {order}"
        ))
    };
    let first = page(0, "");
    let middle = page(total / 2, "");
    let last = page(total.saturating_sub(500), "");
    let filtered_total = one(&format!("SELECT count(*) FROM {from} e WHERE level >= 4"));
    let filtered_page = page(0, "WHERE level >= 4");
    let text = one(&format!("SELECT count(*) FROM {from} e WHERE message ILIKE '%timeout%'"));
    let timeline = one(&format!(
        "WITH b AS (SELECT min(ts) lo, max(ts) hi FROM {from}) SELECT least(999, CAST(floor((ts - lo) * 1000.0 / greatest(hi - lo, 1)) AS BIGINT)) k, level, count(*) FROM {from} e, b WHERE ts IS NOT NULL GROUP BY k, level"
    ));
    let facet = one(&format!(
        "SELECT v, count(*) n FROM (SELECT extra['{facet_key}'] v FROM {from} e) WHERE v IS NOT NULL GROUP BY v ORDER BY n DESC LIMIT 100"
    ));
    let stats = one(&format!(
        "SELECT count(*), min(line), max(line), avg(line), quantile_cont(line, [0.5, 0.95, 0.99]), approx_count_distinct(message) FROM {from} e"
    ));
    eprintln!(
        "  page first {first:.0} ms, middle {middle:.0} ms, last {last:.0} ms; errors total {filtered_total:.0} ms, errors page {filtered_page:.0} ms; text count {text:.0} ms; timeline(1000) {timeline:.0} ms; facet {facet:.0} ms; field stats {stats:.0} ms"
    );
    // Cancellation: interrupt a heavy query after 50 ms and measure how long it takes to stop.
    let handle = c.interrupt_handle();
    let started = Instant::now();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        handle.interrupt();
        Instant::now()
    });
    let r = c.query_row(
        &format!("SELECT count(DISTINCT coalesce(raw, message) || id::VARCHAR) FROM {from} e"),
        [],
        |r| r.get::<_, u64>(0),
    );
    let at = stopper.join().unwrap();
    let ended = Instant::now();
    eprintln!(
        "  cancel: {} after {:.0} ms, {:.0} ms after the interrupt",
        if r.is_err() { "interrupted" } else { "finished before" },
        started.elapsed().as_secs_f64() * 1000.0,
        ended.saturating_duration_since(at).as_secs_f64() * 1000.0
    );
}

/// The most common `extra` key (for the facet).
fn facet_key(batches: &[EventBatch]) -> String {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for b in batches.iter().step_by(7) {
        for r in b.rows() {
            for (k, _) in r.fields() {
                *counts.entry(k.to_string()).or_default() += 1;
            }
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k).unwrap_or_default()
}

#[test]
#[ignore]
fn storage_layouts() {
    let copies: usize = std::env::var("DEBRIEF_BENCH_COPIES").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let threads: usize = std::env::var("DEBRIEF_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let batches = batches(copies);
    let total: u64 = batches.iter().map(|b| b.num_rows() as u64).sum();
    let key = facet_key(&batches);
    let raw_equal = batches.iter().flat_map(|b| b.rows()).filter(|r| r.raw() == r.message()).count();
    eprintln!(
        "{total} events in {} batches; raw == message for {raw_equal}; facet key {key}",
        batches.len()
    );
    let rate = |ms: f64| total as f64 / (ms / 1000.0);

    // A: native table, one appender for the whole import in one transaction, checkpoint at the end.
    for raw_mode in ["raw", "raw only if different"] {
        let dir = TempDir::new("layout-a");
        let c = open(&dir.0.join("data.duckdb"));
        c.execute_batch(DDL).unwrap();
        let peak = Peak::start();
        let (_, ms) = time(|| {
            c.execute_batch("BEGIN").unwrap();
            {
                let mut app = c.appender("events").unwrap();
                for b in &batches {
                    let rb = if raw_mode == "raw" {
                        b.record_batch().clone()
                    } else {
                        raw_if_different(b.record_batch())
                    };
                    app.append_record_batch(rb).unwrap();
                }
                app.flush().unwrap();
            }
            c.execute_batch("COMMIT; CHECKPOINT;").unwrap();
        });
        eprintln!(
            "A native table ({raw_mode}): {:.0} events/s ({ms:.0} ms), peak +{} MB, {:.1} MB on disk",
            rate(ms),
            peak.mb(),
            dir_size(&dir.0) as f64 / 1e6
        );
        if raw_mode == "raw" {
            queries(&c, "events", total, &key);
            // With a time-order position: pages become range lookups.
            let (_, ms) = time(|| {
                c.execute_batch(
                    "CREATE TABLE ord AS SELECT id, row_number() OVER (ORDER BY coalesce(ts, 9223372036854775807), source, line, id) - 1 AS pos FROM events;",
                )
                .unwrap()
            });
            let (_, ms2) = time(|| {
                c.query_row(
                    &format!(
                        "SELECT count(*) FROM (SELECT e.id FROM ord o JOIN events e USING (id) WHERE o.pos BETWEEN {} AND {})",
                        total / 2,
                        total / 2 + 499
                    ),
                    [],
                    |r| r.get::<_, u64>(0),
                )
                .unwrap()
            });
            eprintln!("  pos computed in {ms:.0} ms; middle page by pos {ms2:.0} ms");
        }
    }

    // A': Arrow scan (duckdb-rs ArrowVTab) bulk insert, 16 batches per statement.
    {
        use duckdb::arrow::compute::concat_batches;
        use duckdb::vtab::arrow::{arrow_recordbatch_to_query_params, ArrowVTab};
        let dir = TempDir::new("layout-as");
        let c = open(&dir.0.join("data.duckdb"));
        c.execute_batch(DDL).unwrap();
        c.register_table_function::<ArrowVTab>("arrow").unwrap();
        let sample: Vec<_> = batches.iter().take(64).collect();
        let n: u64 = sample.iter().map(|b| b.num_rows() as u64).sum();
        let (_, ms) = time(|| {
            c.execute_batch("BEGIN").unwrap();
            for chunk in sample.chunks(16) {
                let rbs: Vec<_> = chunk.iter().map(|b| b.record_batch().clone()).collect();
                let rb = concat_batches(&rbs[0].schema(), &rbs).unwrap();
                let p = arrow_recordbatch_to_query_params(rb);
                c.execute("INSERT INTO events SELECT * FROM arrow(?, ?)", p).unwrap();
            }
            c.execute_batch("COMMIT; CHECKPOINT;").unwrap();
        });
        eprintln!(
            "A' Arrow-scan insert ({n} events; leaks every batch in a process-global arena): {:.0} events/s",
            n as f64 / (ms / 1000.0)
        );
    }

    // B: zstd Parquet segments of 16 batches (~131k rows), written by `threads` workers in parallel.
    for threads in [1usize, threads] {
        use parquet::arrow::ArrowWriter;
        use parquet::basic::{Compression, ZstdLevel};
        use parquet::file::properties::WriterProperties;
        let dir = TempDir::new("layout-b");
        let segs = dir.0.join("segments");
        std::fs::create_dir_all(&segs).unwrap();
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let groups: Vec<&[EventBatch]> = batches.chunks(16).collect();
        let peak = Peak::start();
        let (files, ms) = time(|| {
            pool.install(|| {
                use rayon::prelude::*;
                groups
                    .par_iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let path = segs.join(format!("seg-{i:06}.parquet"));
                        let props = WriterProperties::builder()
                            .set_compression(Compression::ZSTD(ZstdLevel::try_new(1).unwrap()))
                            .set_max_row_group_row_count(Some(122_880))
                            .build();
                        let f = std::fs::File::create(&path).unwrap();
                        let mut w = ArrowWriter::try_new(f, g[0].record_batch().schema(), Some(props)).unwrap();
                        for b in *g {
                            w.write(b.record_batch()).unwrap();
                        }
                        w.close().unwrap();
                        path
                    })
                    .collect::<Vec<_>>()
            })
        });
        eprintln!(
            "B Parquet segments, {threads} writers: {:.0} events/s ({ms:.0} ms), peak +{} MB, {:.1} MB on disk, {} segments",
            rate(ms),
            peak.mb(),
            dir_size(&segs) as f64 / 1e6,
            files.len()
        );
        if threads > 1 {
            let c = open(&dir.0.join("data.duckdb"));
            let list = files
                .iter()
                .map(|p| format!("'{}'", p.to_string_lossy().replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(", ");
            let from = format!("read_parquet([{list}])");
            queries(&c, &from, total, &key);
            // Compaction / re-derivation: rewriting every segment (what a profile change costs here).
            let (_, ms) = time(|| {
                c.execute_batch(&format!(
                    "COPY (SELECT * FROM {from}) TO '{}' (FORMAT parquet, COMPRESSION zstd, ROW_GROUP_SIZE 122880)",
                    dir.0.join("compacted.parquet").to_string_lossy()
                ))
                .unwrap()
            });
            eprintln!("  rewrite of all segments by DuckDB: {ms:.0} ms");
        }
    }
}
