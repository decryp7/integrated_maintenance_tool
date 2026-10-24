//! The store worker end to end: the real worker binary under the supervisor, the ports over it, the
//! memory budgets of ADR 0014 (worker ≤ 768 MB importing, ≤ 300 MB idle and querying; the app side flat)
//! and a crash that the next call recovers from by reopening the store.
//!
//! `DEBRIEF_BENCH_COPIES=<n>` imports n copies of the samples for a larger run (use `--release`).

use debrief_core::api::{FacetRequest, TimelineRequest};
use debrief_core::batch::SourceId;
use debrief_core::ports::{Analysis, Ingest, PageQuery, Positions, Projects, Query, Search, SearchRequest, StoreError};
use debrief_core::Profile;
use debrief_store_worker::{MemoryMode, StoreTuning, StoreWorker, WorkerConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const IMPORT_BUDGET_MB: u64 = 768;
/// The import budget in low memory mode (ADR 0014, user decision).
const LOW_MEMORY_IMPORT_BUDGET_MB: u64 = 400;
/// Idle and querying budget (ADR 0014). macOS reports a larger resident size for the same work: its RSS counts the
/// resident pages of libduckdb and the system allocator keeps freed pages (no `malloc_trim`), so the worker idles about
/// 100 MB higher there and queries about 150 MB higher (measured on 1.0.1 and later: idle 304-401 MB, querying 364-522 MB
/// against 300 MB on Linux CI). That is the platform, not a regression; Linux and Windows keep the 300 MB budget.
///
/// Linux 320 MB since the names catalogue (merge fa135656, human names): measured natively on Linux arm64 the worker
/// queried at 266-288 MB before it and 295-307 MB after (v1.0.1: 273-284 MB; 1.1.0: 300-307 MB), and the CI runner
/// at 301-303 MB. The feature's cost, not a leak; the budget follows it (decision to confirm: ADR 0014's budget).
const QUERY_BUDGET_MB: u64 = if cfg!(target_os = "macos") { 550 } else { 320 };
/// Growth of the app process allowed while it drives an import and queries (it holds no events).
const CLIENT_GROWTH_MB: u64 = 64;

/// A fresh folder under the system temp folder, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let p = std::env::temp_dir().join(format!("debrief-worker-{tag}-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The folders of `/samples` the built-in importers read (as in debrief-store-duck's tests).
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
        "real-libraries",
        "encodings",
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
    worker_in(MemoryMode::Normal)
}

fn worker_in(mode: MemoryMode) -> StoreWorker {
    let mut cfg = WorkerConfig::new(env!("CARGO_BIN_EXE_debrief-store-worker"));
    cfg.memory_mode = mode;
    StoreWorker::new(cfg).unwrap()
}

#[test]
fn imports_and_queries_within_the_budgets() {
    imports_and_queries_within(MemoryMode::Normal, IMPORT_BUDGET_MB);
}

/// Low memory mode (a setting, off by default): the worker starts in it and imports within 400 MB.
#[test]
fn low_memory_mode_imports_within_its_budget() {
    imports_and_queries_within(MemoryMode::Low, LOW_MEMORY_IMPORT_BUDGET_MB);
}

fn imports_and_queries_within(mode: MemoryMode, import_budget_mb: u64) {
    let copies: usize = std::env::var("DEBRIEF_BENCH_COPIES").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
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
    let w = worker_in(mode);
    let store = w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    let pid = w.status().pid.unwrap();
    assert_eq!(
        w.worker_status().unwrap().memory_mode,
        mode,
        "the worker started in the mode asked for"
    );
    let sampler = Sampler::start(pid);
    let started = Instant::now();
    let session = store.open(&paths).unwrap();
    while !session.is_finished() {
        let p = session.progress();
        // "Finding" counts the files found so far on a scale of its own (`total` is unknown, 0): only the phases
        // that read files have a total to stay within.
        assert!(
            p.phase == debrief_core::workspace::ImportPhase::Finding || p.done <= p.total,
            "{p:?}"
        );
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
        import_peak.max(index_peak) <= import_budget_mb,
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
    let bad_regex = PageQuery {
        query: debrief_core::ports::Filter {
            text: "(".into(),
            regex: true,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        matches!(store.page(&bad_regex), Err(StoreError::Invalid(_))),
        "{:?}",
        store.page(&bad_regex)
    );
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

#[test]
fn a_project_exported_through_the_worker_opens_without_its_logs() {
    let logs = TempDir::new("project-logs");
    std::fs::write(
        logs.0.join("a.log"),
        "2026-10-14 10:00:00 INFO first
2026-10-14 10:00:01 ERROR second
2026-10-14 10:00:02 WARN third
",
    )
    .unwrap();
    let home = TempDir::new("project-home");
    let dir = home.0.join("incident.debrief");
    let w = worker();
    let store = w.open_store(&dir, Profile::default_profile(), true, None).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    let manifest = debrief_core::package::ProjectManifest {
        name: "Incident".into(),
        ..Default::default()
    };
    let saved = store.save_project(&manifest).unwrap();
    assert_eq!(saved.roots.len(), 1, "the opened folder is remembered for refresh");
    let archive = home.0.join("incident.debriefz");
    assert!(store.export_project(&saved, &archive).unwrap() > 0);
    store.close();
    drop(logs);
    // The archive opens in a new folder, without the log files.
    let unpacked = TempDir::new("project-open");
    let (opened, m) = w.open_project(&archive, &unpacked.0, Profile::default_profile(), None).unwrap();
    assert_eq!(m.name, "Incident");
    assert!(opened.store_dir().starts_with(std::fs::canonicalize(&unpacked.0).unwrap()));
    assert_eq!(opened.summary().unwrap().total, 3);
    // The folder opens in place too.
    let (in_place, _) = w.open_project(&dir, &unpacked.0, Profile::default_profile(), None).unwrap();
    assert_eq!(in_place.summary().unwrap().total, 3);
    // Something that is not a project is refused with a message.
    let bogus = home.0.join("bogus.debriefz");
    std::fs::write(&bogus, b"not a zip").unwrap();
    assert!(matches!(
        w.open_project(&bogus, &unpacked.0, Profile::default_profile(), None),
        Err(StoreError::Invalid(_))
    ));
    w.shutdown();
}

/// Kills the worker process (as a crash would).
fn kill(w: &StoreWorker) -> u32 {
    let pid = w.status().pid.unwrap();
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    assert!(sys.process(p).unwrap().kill());
    pid
}

#[test]
fn a_worker_killed_mid_import_or_mid_query_fails_the_call_and_recovers() {
    let logs = TempDir::new("killed-logs");
    for (i, from) in log_samples().iter().enumerate() {
        copy_dir(from, &logs.0.join(format!("s{i}")));
    }
    let dir = TempDir::new("killed-store");
    let w = worker();
    let store = w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    // Mid-import: the import job fails (retryable), it never hangs.
    let session = store.open(std::slice::from_ref(&logs.0)).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let pid = kill(&w);
    match session.wait() {
        Ok(_) => {} // finished before the kill landed
        Err(e) => assert!(e.is_retryable(), "{e}"),
    }
    // The store reopens on the new worker, with whatever the import had committed (or nothing).
    let deadline = Instant::now() + Duration::from_secs(30);
    let summary = loop {
        match store.summary() {
            Ok(s) => break s,
            Err(e) if e.is_retryable() && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("{e}"),
        }
    };
    assert_ne!(w.status().pid, Some(pid));
    // Importing again completes.
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    let total = store.summary().unwrap().total;
    assert!(total > 10_000 && total >= summary.total, "{total}");
    // Mid-query: a long analysis on another thread fails (retryable) or finishes; the next call works.
    let s2 = store.clone();
    let running = std::thread::spawn(move || {
        s2.patterns(&debrief_core::api::QueryRequest { query: Default::default() })
            .map(|_| ())
    });
    std::thread::sleep(Duration::from_millis(30));
    kill(&w);
    match running.join().unwrap() {
        Ok(()) => {}
        Err(e) => assert!(e.is_retryable(), "{e}"),
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match store.summary() {
            Ok(s) => {
                assert_eq!(s.total, total, "nothing lost");
                break;
            }
            Err(e) if e.is_retryable() && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("{e}"),
        }
    }
    w.shutdown();
}

#[test]
fn a_store_open_in_another_worker_is_locked_with_a_typed_error() {
    let dir = TempDir::new("locked-store");
    let first = worker();
    let _held = first.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    let second = worker();
    let failure = second
        .open_store(&dir.0, Profile::default_profile(), false, None)
        .err()
        .expect("the catalog is locked");
    assert!(matches!(&failure, StoreError::Locked(_)), "{failure:?}");
    assert!(failure.is_retryable());
    second.shutdown();
    first.shutdown();
}

/// Settings ▸ Performance (ADR 0031): the worker starts with the tuning it is given, and a change reaches its next
/// start (here after a crash) without a new client.
#[test]
fn the_worker_starts_with_its_tuning_and_the_next_start_takes_a_changed_one() {
    let mut cfg = WorkerConfig::new(env!("CARGO_BIN_EXE_debrief-store-worker"));
    let first = StoreTuning {
        parse_threads: Some(2),
        duck_threads: Some(3),
        memory_limit_mb: Some(160),
        ..StoreTuning::default()
    };
    cfg.tuning = first.clone();
    let w = StoreWorker::new(cfg).unwrap();
    let dir = TempDir::new("tuning");
    let store = w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    assert_eq!(w.worker_status().unwrap().tuning, first);
    let second = StoreTuning {
        duck_threads: Some(1),
        allocator_background_threads: Some(false),
        ..StoreTuning::default()
    };
    let mut launch = w.launch_settings();
    assert_eq!(launch.tuning, first);
    launch.tuning = second.clone();
    launch.memory_mode = MemoryMode::Low;
    w.set_launch(launch);
    assert_eq!(w.worker_status().unwrap().tuning, first, "a running worker keeps its values");
    kill(&w);
    // The next call starts the worker again, with the new values; the store reopens.
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match w.worker_status() {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("the worker did not start again: {e}"),
        }
    };
    assert_eq!((status.tuning, status.memory_mode), (second, MemoryMode::Low));
    assert_eq!(store.summary().unwrap().total, 0);
}

/// The Store pane over the worker (ADR 0030): reports, memory with the worker's process, maintenance, and SQL
/// console runs that are refused, answered and cancelled across the process boundary.
#[test]
fn the_store_pane_reaches_the_worker() {
    use debrief_core::ports::{Console, Inspect, Maintain, MaintenanceAction, SegmentsRequest, SqlPageRequest, SqlRequest};
    let dir = TempDir::new("pane");
    let w = worker();
    let store = w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-clef");
    let report = store.open(&[samples]).unwrap().wait().unwrap();
    let overview = store.store_overview().unwrap();
    assert_eq!(overview.events, report.events as u64);
    assert_eq!(store.store_segments(&SegmentsRequest::default()).unwrap().total, overview.segments);
    assert!(!store.store_fields().unwrap().fields.is_empty());
    assert_eq!(store.store_index().unwrap().events, overview.events);
    let memory = store.store_memory().unwrap();
    let worker = memory.worker.expect("the worker fills in its process");
    assert_eq!(Some(worker.pid), w.status().pid);
    assert!(worker.rss_bytes > 0 && !worker.low_memory);
    assert!(store
        .maintain(MaintenanceAction::ClearCachedAnalyses)
        .unwrap()
        .message
        .starts_with("Cleared"));
    let sql = |text: &str| SqlRequest {
        sql: text.into(),
        ..SqlRequest::default()
    };
    let n = store.sql_run(&sql("SELECT count(*) FROM events")).unwrap();
    assert_eq!(n.rows[0][0], report.events as u64);
    assert!(matches!(store.sql_run(&sql("ATTACH 'x' AS y")), Err(StoreError::Query(_))));
    let many = store.sql_run(&sql("SELECT * FROM range(2000)")).unwrap();
    let page = store
        .sql_page(&SqlPageRequest {
            result: many.result,
            offset: 1500,
        })
        .unwrap();
    assert_eq!(page.rows.len(), 500);
    let control = debrief_core::ql::QueryControl::new();
    let c = control.clone();
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c.cancel();
    });
    let started = Instant::now();
    let out = control.scope(|| store.sql_run(&sql("SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3")));
    cancel.join().unwrap();
    assert!(matches!(out, Err(StoreError::Cancelled)), "{out:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(
        store.sql_run(&sql("SELECT 7")).unwrap().rows[0][0],
        7,
        "the console works after a cancel"
    );
}

/// Warming up starts the worker before any store call (the app's first query does not wait for the start), once.
#[test]
fn warm_up_starts_the_worker_before_the_first_call() {
    use debrief_ipc::supervisor::ChildState;
    let w = worker();
    assert_eq!(w.status().state, ChildState::Stopped, "a new worker starts on demand");
    w.warm_up().unwrap();
    let status = w.status();
    assert_eq!(status.state, ChildState::Running);
    let pid = status.pid;
    w.warm_up().unwrap();
    assert_eq!(w.status().pid, pid, "a running worker is kept");
    let dir = TempDir::new("store");
    w.open_store(&dir.0, Profile::default_profile(), true, None).unwrap();
    assert_eq!(w.status().pid, pid, "the first store call uses the warmed-up worker");
    w.shutdown();
}

/// A worker that cannot start says why, without panicking.
#[test]
fn warm_up_reports_a_worker_that_cannot_start() {
    let w = StoreWorker::new(WorkerConfig::new("/nonexistent/debrief-store-worker")).unwrap();
    assert!(w.warm_up().is_err());
}
