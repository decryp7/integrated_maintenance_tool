//! The store worker end to end: the real worker binary under the supervisor, the ports over it, the
//! memory budgets of ADR 0014 (worker ≤ 1 GB importing, ≤ 300 MB idle and querying; the app side flat)
//! and a crash that the next call recovers from by reopening the store.
//!
//! `IMT_BENCH_COPIES=<n>` imports n copies of the samples for a larger run (use `--release`).

use imt_core::api::{FacetRequest, TimelineRequest};
use imt_core::batch::SourceId;
use imt_core::ports::{Ingest, PageQuery, Positions, Query, Search, SearchRequest, StoreError};
use imt_core::Profile;
use imt_store_worker::{StoreWorker, WorkerConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const IMPORT_BUDGET_MB: u64 = 1024;
const QUERY_BUDGET_MB: u64 = 300;
/// Growth of the app process allowed while it drives an import and queries (it holds no events).
const CLIENT_GROWTH_MB: u64 = 64;

/// A fresh folder under the system temp folder, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let p = std::env::temp_dir().join(format!("imt-worker-{tag}-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The folders of `/samples` the built-in importers read (as in imt-store-duck's tests).
fn log_samples() -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "profiling",
        "configs",
        "share",
        "live",
        "windows",
        "web",
        "linux",
        "otel",
        "profiles",
    ];
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| SKIP.iter().any(|s| n == *s)))
        .collect();
    v.sort();
    v
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

fn rss_of(pid: u32) -> u64 {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map_or(0, |p| p.memory())
}

fn own_rss() -> u64 {
    rss_of(std::process::id())
}

/// Polls a process' RSS and keeps the peak of the current phase.
struct Sampler {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    pid: u32,
}

impl Sampler {
    fn start(pid: u32) -> Sampler {
        let peak = Arc::new(AtomicU64::new(rss_of(pid)));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss_of(pid), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Sampler { peak, stop, pid }
    }

    fn take_mb(&self) -> u64 {
        let now = rss_of(self.pid);
        self.peak.swap(now, Ordering::Relaxed).max(now) >> 20
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn worker() -> StoreWorker {
    StoreWorker::new(WorkerConfig::new(env!("CARGO_BIN_EXE_imt-store-worker"))).unwrap()
}

#[test]
fn imports_and_queries_within_the_budgets() {
    let copies: usize = std::env::var("IMT_BENCH_COPIES").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let data = TempDir::new("data");
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
    let dir = TempDir::new("store");
    let client_before = own_rss() >> 20;
    let w = worker();
    let store = w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    let pid = w.status().pid.unwrap();
    let sampler = Sampler::start(pid);
    let started = Instant::now();
    let session = store.open(&paths).unwrap();
    while !session.is_finished() {
        let p = session.progress();
        assert!(p.done <= p.total);
        std::thread::sleep(Duration::from_millis(50));
    }
    let report = session.wait().unwrap();
    let elapsed = started.elapsed().as_secs_f64();
    let import_peak = sampler.take_mb();
    let indexed = Instant::now();
    while !store.index_status().unwrap().ready && indexed.elapsed() < Duration::from_secs(600) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let index_peak = sampler.take_mb();
    std::thread::sleep(Duration::from_millis(300));
    let idle = rss_of(pid) >> 20;
    sampler.take_mb();
    // Printed before the queries, so a failing query still reports the import.
    eprintln!(
        "worker: import {} events in {elapsed:.2} s ({:.0}/s); peak RSS importing {import_peak} MB, indexing {index_peak} MB, idle {idle} MB",
        report.events,
        report.events as f64 / elapsed
    );
    // Queries.
    let summary = store.summary().unwrap();
    assert_eq!(summary.total, report.events);
    let page = store
        .page(&PageQuery {
            limit: 200,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.rows.len(), 200);
    assert_eq!(store.ids_at(0, 200).unwrap(), page.rows.iter().map(|r| r.id).collect::<Vec<_>>());
    let t = store
        .timeline(&TimelineRequest {
            query: Default::default(),
            buckets: 400,
            always_show_lifecycle: true,
        })
        .unwrap();
    assert_eq!(t.histogram.len(), 400);
    let f = store
        .facet(&FacetRequest {
            query: Default::default(),
            field: "@level".into(),
            limit: 10,
        })
        .unwrap();
    assert!(!f.values.is_empty());
    let hits = store
        .search(&SearchRequest {
            text: "timeout".into(),
            limit: 10,
        })
        .unwrap();
    assert!(hits.total > 0 && hits.exact);
    let query_peak = sampler.take_mb();
    let client_after = own_rss() >> 20;
    eprintln!(
        "worker: imported {} events in {elapsed:.2} s ({:.0}/s); peak RSS importing {import_peak} MB, indexing {index_peak} MB, idle {idle} MB, querying {query_peak} MB; client RSS {client_before} -> {client_after} MB",
        report.events,
        report.events as f64 / elapsed
    );
    assert!(
        import_peak.max(index_peak) <= IMPORT_BUDGET_MB,
        "importing: {import_peak}/{index_peak} MB"
    );
    assert!(idle <= QUERY_BUDGET_MB, "idle: {idle} MB");
    assert!(query_peak <= QUERY_BUDGET_MB, "querying: {query_peak} MB");
    assert!(
        client_after.saturating_sub(client_before) <= CLIENT_GROWTH_MB,
        "the app side grew from {client_before} to {client_after} MB"
    );
}

#[test]
fn errors_pass_through_and_a_crash_is_recovered() {
    let logs = TempDir::new("crash-logs");
    std::fs::write(
        logs.0.join("a.log"),
        "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 ERROR second\n",
    )
    .unwrap();
    let dir = TempDir::new("crash-store");
    let w = worker();
    let store = w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, 2);
    // Typed errors come back as they were.
    assert!(matches!(store.remove_source(SourceId(999)), Err(StoreError::NotFound(_))));
    assert!(matches!(store.restore(&Default::default()), Err(StoreError::Unsupported { .. })));
    // The worker dies: the next call fails (retryable) or already runs on a new worker; then all is back.
    let pid = w.status().pid.unwrap();
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    assert!(sys.process(p).unwrap().kill());
    std::thread::sleep(Duration::from_millis(200));
    match store.summary() {
        Ok(s) => assert_eq!(s.total, 2),
        Err(e) => assert!(e.is_retryable(), "{e}"),
    }
    let back = store.summary().unwrap();
    assert_eq!(back.total, 2, "the restarted worker reopened the store");
    assert_ne!(w.status().pid, Some(pid));
    assert!(w.status().restarts >= 1);
    // A second store in the same worker.
    let other = TempDir::new("crash-other");
    let second = w.open_store(&other.0, Profile::default_profile(), true, None).unwrap();
    assert_eq!(second.summary().unwrap().total, 0);
    assert_eq!(w.worker_status().unwrap().stores, 2);
    w.shutdown();
}
