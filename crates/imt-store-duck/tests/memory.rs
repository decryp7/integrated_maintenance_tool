//! Memory budget (ADR 0014 §2): importing stays under 600 MB resident whatever is imported. A sampler
//! thread polls the process RSS every 20 ms; the test fails above the budget and prints the ingest rate.
//!
//! `IMT_BENCH_COPIES=<n>` imports n copies of the samples (each in its own folder) for a larger run;
//! measure with `--release` for representative speed.

mod common;

use common::{log_samples, TempDir};
use imt_core::ports::Query;
use imt_core::Profile;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Worker budget while importing (ADR 0014).
const IMPORT_BUDGET_MB: u64 = 600;

fn rss() -> u64 {
    memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
}

/// Polls the RSS until stopped; returns the peak.
struct Sampler {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    fn start() -> Sampler {
        let peak = Arc::new(AtomicU64::new(rss()));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Sampler {
            peak,
            stop,
            thread: Some(thread),
        }
    }

    fn peak_mb(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.peak.load(Ordering::Relaxed).max(rss()) >> 20
    }
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

#[test]
fn importing_the_samples_stays_within_the_memory_budget() {
    let copies: usize = std::env::var("IMT_BENCH_COPIES").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let data = TempDir::new("bench-data");
    let paths: Vec<PathBuf> = if copies <= 1 {
        log_samples()
    } else {
        (0..copies)
            .map(|i| {
                let d = data.0.join(format!("copy{i}"));
                for p in log_samples() {
                    copy_dir(&p, &d.join(p.file_name().unwrap()));
                }
                d
            })
            .collect()
    };
    let bytes: u64 = walk_size(&paths);
    let dir = TempDir::new("bench");
    let before = rss() >> 20;
    let sampler = Sampler::start();
    let started = Instant::now();
    let (store, report) = common::store_with(&dir, Profile::default_profile(), &paths).unwrap();
    let elapsed = started.elapsed();
    let peak = sampler.peak_mb();
    let total = store.summary().unwrap().total;
    let rate = report.events as f64 / elapsed.as_secs_f64();
    eprintln!(
        "imported {} events ({:.1} MB) in {:.2} s: {:.0} events/s, {:.1} MB/s; RSS before {before} MB, peak {peak} MB",
        report.events,
        bytes as f64 / 1e6,
        elapsed.as_secs_f64(),
        rate,
        bytes as f64 / 1e6 / elapsed.as_secs_f64()
    );
    assert_eq!(total, report.events);
    // The same files parsed into batches that are dropped: the reading side alone.
    let started = Instant::now();
    let parsed = parse_only(&paths);
    eprintln!(
        "parsing alone: {parsed} events in {:.2} s ({:.0} events/s)",
        started.elapsed().as_secs_f64(),
        parsed as f64 / started.elapsed().as_secs_f64()
    );
    assert!(
        peak <= IMPORT_BUDGET_MB,
        "peak RSS {peak} MB is over the import budget of {IMPORT_BUDGET_MB} MB"
    );
}

/// Imports into a sink that only counts.
fn parse_only(paths: &[PathBuf]) -> usize {
    use imt_core::batch::{PendingBatch, SourceId};
    use imt_core::ingest::{self, BatchSink, ImportEnv, NewSource, SourceReport};
    struct Count(usize, u32);
    impl BatchSink for Count {
        fn begin_source(&mut self, _s: &NewSource) -> anyhow::Result<SourceId> {
            self.1 += 1;
            Ok(SourceId(self.1))
        }
        fn write(&mut self, b: PendingBatch) -> anyhow::Result<()> {
            self.0 += b.len();
            Ok(())
        }
        fn finish_source(&mut self, _r: &SourceReport) -> anyhow::Result<()> {
            Ok(())
        }
        fn abort_source(&mut self, _id: SourceId) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let registry = imt_core::PluginRegistry::new();
    let profile = registry.compile_profile(Profile::default_profile()).unwrap();
    let progress = imt_core::workspace::ImportProgress::default();
    let env = ImportEnv {
        importers: registry.importers(),
        profile: &profile,
        schema: Arc::new(ingest::schema_for(&profile)),
        cancel: progress.cancel_flag(),
    };
    let canonical: Vec<PathBuf> = paths.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect();
    let files = ingest::files_to_read(&canonical, &canonical, &profile, &Default::default(), &Default::default());
    let mut sink = Count(0, 0);
    ingest::import_files(&env, &files, &Default::default(), &progress, &mut sink);
    sink.0
}

fn walk_size(paths: &[PathBuf]) -> u64 {
    paths
        .iter()
        .map(|p| {
            if p.is_dir() {
                let entries: Vec<PathBuf> = std::fs::read_dir(p).unwrap().map(|e| e.unwrap().path()).collect();
                walk_size(&entries)
            } else {
                std::fs::metadata(p).map_or(0, |m| m.len())
            }
        })
        .sum()
}
