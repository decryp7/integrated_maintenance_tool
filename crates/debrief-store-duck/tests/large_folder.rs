//! A very large folder imports with live progress from the first moments (phase, counts), and timings of
//! each phase on a synthetic tree. The tree is generated here (never real data); `DEBRIEF_BENCH_FILES=<n>`
//! sets its size (default 3,000; the timing run uses 100,000).

mod common;

use common::TempDir;
use debrief_core::ports::Ingest;
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::{DuckStore, StoreOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LINE: &str = "2026-10-14 10:00:00 INFO synthetic line\n";

/// `n` small logs over nested folders, a few big ones and some zips.
fn tree(root: &Path, n: usize) {
    for i in 0..n {
        let dir = root.join(format!("a{}/b{}/c{}", i % 20, (i / 20) % 20, (i / 400) % 50));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("f{i}.log")), LINE.repeat(3)).unwrap();
    }
    let big = root.join("big");
    std::fs::create_dir_all(&big).unwrap();
    for i in 0..3 {
        std::fs::write(big.join(format!("big{i}.log")), LINE.repeat(200_000)).unwrap();
    }
    for i in 0..5 {
        let f = std::fs::File::create(root.join(format!("arch{i}.zip"))).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for j in 0..5 {
            z.start_file(format!("e{j}.log"), zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(LINE.repeat(10).as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
}

fn files() -> usize {
    std::env::var("DEBRIEF_BENCH_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000)
}

fn store(dir: &TempDir, threads: usize) -> DuckStore {
    let opts = StoreOptions {
        parse_threads: threads,
        ..StoreOptions::default()
    };
    DuckStore::create_with(&dir.0, Profile::default_profile(), PluginRegistry::new(), opts).unwrap()
}

#[test]
fn progress_starts_at_once_and_the_found_count_grows() {
    let data = TempDir::new("large-data");
    tree(&data.0, files());
    let dir = TempDir::new("large-store");
    let store = store(
        &dir,
        std::env::var("DEBRIEF_PARSE_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4),
    );
    let paths: Vec<PathBuf> = vec![data.0.clone()];
    let started = Instant::now();
    let session = store.open(&paths).unwrap();
    let mut first_running: Option<Duration> = None;
    let mut first_total: Option<Duration> = None;
    let mut seen: Vec<(Duration, String, usize, usize, u64, bool)> = Vec::new();
    while !session.is_finished() {
        let p = session.progress();
        let t = started.elapsed();
        if p.running && first_running.is_none() {
            first_running = Some(t);
        }
        if p.total > 0 && first_total.is_none() {
            first_total = Some(t);
        }
        seen.push((t, format!("{:?}", p.phase), p.done, p.total, p.bytes_total, p.finding));
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = session.wait().unwrap();
    eprintln!(
        "files={} first running snapshot: {first_running:?}; total known: {first_total:?}; whole import {:?}; events {}",
        files(),
        started.elapsed(),
        report.events
    );
    let mut last = None;
    for s in &seen {
        if last.as_ref() != Some(&s.1) {
            eprintln!("  phase {} from {:?} (done {} total {})", s.1, s.0, s.2, s.3);
            last = Some(s.1.clone());
        }
    }
    assert!(
        first_running.is_some_and(|t| t < Duration::from_millis(250)),
        "no progress within 250 ms: {first_running:?}"
    );
    // While finding, the count of found files only grows and does so more than once on a large tree.
    let finding: Vec<usize> = seen.iter().filter(|s| s.1 == "Finding").map(|s| s.2).collect();
    assert!(finding.windows(2).all(|w| w[0] <= w[1]), "not monotonic: {finding:?}");
    if files() >= 50_000 {
        let mut distinct = finding.clone();
        distinct.dedup();
        assert!(distinct.len() > 3, "the found count did not visibly grow: {distinct:?}");
    }
    // The phases come in order and each is shown with its own counts.
    // Files are read while they are found (ADR 0051): there is no separate checking phase any more.
    let order = ["Idle", "Finding", "Importing", "Finishing"];
    let ranks: Vec<usize> = seen
        .iter()
        .filter(|s| s.1 != "Idle" || s.2 == 0)
        .map(|s| order.iter().position(|o| *o == s.1).unwrap())
        .collect();
    assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "phases out of order: {ranks:?}");
    assert!(
        seen.iter().any(|s| s.1 == "Importing" && !s.5 && s.3 == files() + 5 + 3),
        "the importing phase knows the total once the walk ended"
    );
    assert!(report.events > 0);
}
