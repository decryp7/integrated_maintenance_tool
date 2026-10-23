//! Segmented (multi-connection) downloads: the engine against a controllable in-memory source, and the Hugging
//! Face adapter's range support against a local throttled server. Never the real network.

mod common;
use common::*;
use debrief_ai::models::*;
use debrief_ai::AiError;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Delay = Box<dyn Fn(u64, usize) -> Duration + Send + Sync>;
type Reject = Box<dyn Fn(u64, usize, usize) -> Option<AiError> + Send + Sync>;

/// An in-memory host. `delay(start, call)` is the pause before each 16 KiB buffer of a request;
/// `reject(start, call, in_flight)` may refuse a request.
struct Fake {
    data: Arc<Vec<u8>>,
    ranges: bool,
    delay: Option<Delay>,
    reject: Option<Reject>,
    on_data: Option<Box<dyn Fn(u64) + Send + Sync>>,
    calls: AtomicUsize,
    in_flight: AtomicUsize,
    served: AtomicU64,
    log: Mutex<Vec<(u64, u64, usize)>>,
}

impl Fake {
    fn new(data: Vec<u8>) -> Fake {
        Fake {
            data: Arc::new(data),
            ranges: true,
            delay: None,
            reject: None,
            on_data: None,
            calls: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            served: AtomicU64::new(0),
            log: Mutex::new(vec![]),
        }
    }
}

struct InFlight<'a>(&'a AtomicUsize);
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ModelSource for Fake {
    fn describe(&self, repo: &str, file: &str) -> debrief_ai::Result<RemoteFile> {
        Ok(RemoteFile {
            repo: repo.into(),
            file: file.into(),
            size: self.data.len() as u64,
            sha256: sha256_hex(&self.data),
            license: None,
            gated: false,
        })
    }

    fn fetch(
        &self,
        r: &RemoteFile,
        offset: u64,
        sink: &mut dyn FnMut(&[u8]) -> debrief_ai::Result<()>,
        cancel: &AtomicBool,
    ) -> debrief_ai::Result<()> {
        self.fetch_range(r, offset, r.size, sink, cancel)
    }

    fn supports_ranges(&self, _: &RemoteFile) -> bool {
        self.ranges
    }

    fn fetch_range(
        &self,
        _: &RemoteFile,
        start: u64,
        end: u64,
        sink: &mut dyn FnMut(&[u8]) -> debrief_ai::Result<()>,
        cancel: &AtomicBool,
    ) -> debrief_ai::Result<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        let _g = InFlight(&self.in_flight);
        self.log.lock().unwrap().push((start, end, now));
        if let Some(e) = self.reject.as_ref().and_then(|f| f(start, call, now)) {
            return Err(e);
        }
        let mut at = start as usize;
        while at < end as usize {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            if let Some(d) = &self.delay {
                std::thread::sleep(d(start, call));
            }
            let to = (at + 16 * 1024).min(end as usize);
            sink(&self.data[at..to])?;
            let total = self.served.fetch_add((to - at) as u64, Ordering::SeqCst) + (to - at) as u64;
            if let Some(f) = &self.on_data {
                f(total);
            }
            at = to;
        }
        Ok(())
    }
}

struct Disk;
impl DiskSpace for Disk {
    fn available_bytes(&self, _: &Path) -> Option<u64> {
        Some(u64::MAX)
    }
}

fn opts() -> DownloadOptions {
    DownloadOptions {
        max_bytes: 1 << 30,
        margin_bytes: 0,
        attempts: 3,
        backoff: Duration::from_millis(2),
        connections: 4,
        segment_threshold: 1 << 20,
        min_block: 64 * 1024,
        host_gate: None,
    }
}

fn download(src: &dyn ModelSource, dir: &Path, o: DownloadOptions, p: &Progress) -> debrief_ai::Result<std::path::PathBuf> {
    let remote = src.describe("o/r", "big.gguf")?;
    Downloader {
        source: src,
        disk: &Disk,
        opts: o,
    }
    .download(&remote, dir, p)
}

const SIZE: usize = 6 * 1024 * 1024 + 12345;

#[test]
fn ranges_in_parallel_reassemble_to_the_original_bytes_and_hash() {
    let src = Fake::new(blob(SIZE));
    let dir = tempfile::tempdir().unwrap();
    let p = Progress::new();
    let path = download(&src, dir.path(), opts(), &p).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), *src.data);
    // Nothing is left behind, and the file was requested in many ranges that never overlap more than the steals.
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("big.gguf")]);
    let log = src.log.lock().unwrap();
    assert!(log.len() > 4, "{} requests", log.len());
    assert!(log.iter().map(|l| l.2).max().unwrap() >= 2, "connections ran in parallel");
    assert!(log.iter().map(|l| l.2).max().unwrap() <= 4);
    assert_eq!(p.snapshot().bytes, SIZE as u64);
}

#[test]
fn the_connection_setting_is_capped_at_eight() {
    let mut src = Fake::new(blob(SIZE));
    src.delay = Some(Box::new(|_, _| Duration::from_millis(2)));
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts();
    o.connections = 64;
    download(&src, dir.path(), o, &Progress::new()).unwrap();
    let peak = src.log.lock().unwrap().iter().map(|l| l.2).max().unwrap();
    assert!((2..=8).contains(&peak), "{peak}");
}

#[test]
fn a_host_without_ranges_or_a_small_file_is_one_stream() {
    let mut src = Fake::new(blob(SIZE));
    src.ranges = false;
    let dir = tempfile::tempdir().unwrap();
    let path = download(&src, dir.path(), opts(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *src.data);
    assert_eq!(src.log.lock().unwrap().len(), 1);

    let small = Fake::new(blob(SIZE));
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts();
    o.segment_threshold = SIZE as u64 + 1;
    download(&small, dir.path(), o, &Progress::new()).unwrap();
    assert_eq!(small.log.lock().unwrap().len(), 1);

    let single = Fake::new(blob(SIZE));
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts();
    o.connections = 1;
    download(&single, dir.path(), o, &Progress::new()).unwrap();
    assert_eq!(single.log.lock().unwrap().len(), 1);
}

#[test]
fn a_failing_range_retries_on_its_own_and_the_rest_is_untouched() {
    let mut src = Fake::new(blob(SIZE));
    // The third and fourth requests drop at once; the engine retries those ranges after a short backoff.
    src.reject = Some(Box::new(|_, call, _| {
        (call == 2 || call == 3).then(|| AiError::Network("reset".into()))
    }));
    let dir = tempfile::tempdir().unwrap();
    let path = download(&src, dir.path(), opts(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *src.data);
}

#[test]
fn a_range_that_keeps_failing_fails_the_download_and_keeps_what_arrived() {
    let mut src = Fake::new(blob(SIZE));
    src.reject = Some(Box::new(|start, _, _| (start >= 4 << 20).then(|| AiError::Network("reset".into()))));
    let dir = tempfile::tempdir().unwrap();
    let r = download(&src, dir.path(), opts(), &Progress::new());
    assert!(matches!(r, Err(AiError::Network(_))), "{r:?}");
    assert!(dir.path().join("big.gguf.part").exists() && dir.path().join("big.gguf.part.ranges").exists());
    assert!(!dir.path().join("big.gguf").exists());
}

#[test]
fn a_definite_error_is_not_retried() {
    let mut src = Fake::new(blob(SIZE));
    src.reject = Some(Box::new(|_, _, _| Some(AiError::Refused("no".into()))));
    let dir = tempfile::tempdir().unwrap();
    let r = download(&src, dir.path(), opts(), &Progress::new());
    assert!(matches!(r, Err(AiError::Refused(_))), "{r:?}");
}

#[test]
fn idle_connections_steal_the_tail_of_a_slow_one() {
    let mut src = Fake::new(blob(8 << 20));
    // The very first request crawls (20 ms per 16 KiB); the others are fast.
    src.delay = Some(Box::new(
        |_, call| if call == 0 { Duration::from_millis(20) } else { Duration::ZERO },
    ));
    let dir = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let path = download(&src, dir.path(), opts(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *src.data);
    let blocks = (8usize << 20) / (256 * 1024);
    let calls = src.calls.load(Ordering::SeqCst);
    assert!(calls > blocks, "{calls} requests for {blocks} blocks: the slow block was split");
    // Without stealing the slow 256 KiB block alone would take 16 x 20 ms = 320 ms after everything else is done.
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn throttling_lowers_the_connections() {
    let mut src = Fake::new(blob(SIZE));
    // The host allows one connection at a time and says 429 to the others.
    src.delay = Some(Box::new(|_, _| Duration::from_millis(1)));
    src.reject = Some(Box::new(|_, _, now| {
        (now > 1).then(|| {
            AiError::Connection(
                debrief_ai::net::NetError::new(debrief_ai::net::NetKind::Http(429), "h", "slow down").with_retry_after(Some("0")),
            )
        })
    }));
    let dir = tempfile::tempdir().unwrap();
    let path = download(&src, dir.path(), opts(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *src.data);
    let log = src.log.lock().unwrap();
    assert!(log.iter().take(4).any(|l| l.2 > 1), "it started with several connections");
    let late = &log[log.len() - log.len() / 5..];
    assert!(late.iter().all(|l| l.2 == 1), "the last requests ran one at a time: {late:?}");
}

#[test]
fn cancelling_stops_every_connection_quickly_and_keeps_the_resumable_state() {
    let mut src = Fake::new(blob(SIZE));
    src.delay = Some(Box::new(|_, _| Duration::from_millis(5)));
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Progress::new());
    let (p2, d2) = (p.clone(), dir.path().to_path_buf());
    let src = Arc::new(src);
    let s2 = src.clone();
    let h = std::thread::spawn(move || download(&*s2, &d2, opts(), &p2));
    while p.snapshot().bytes < 512 * 1024 {
        std::thread::sleep(Duration::from_millis(2));
    }
    let t = Instant::now();
    p.cancel();
    let r = h.join().unwrap();
    assert!(t.elapsed() < Duration::from_millis(250), "stopped after {:?}", t.elapsed());
    assert!(matches!(r, Err(AiError::Cancelled)), "{r:?}");
    assert!(dir.path().join("big.gguf.part").exists() && dir.path().join("big.gguf.part.ranges").exists());
    assert!(src.in_flight.load(Ordering::SeqCst) == 0);
}

#[test]
fn an_interrupted_download_resumes_only_the_missing_ranges() {
    let data = blob(SIZE);
    let mut first = Fake::new(data.clone());
    first.delay = Some(Box::new(|_, _| Duration::from_millis(2)));
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Progress::new());
    let (p2, d2) = (p.clone(), dir.path().to_path_buf());
    let first = Arc::new(first);
    let f2 = first.clone();
    let h = std::thread::spawn(move || download(&*f2, &d2, opts(), &p2));
    while p.snapshot().bytes < (SIZE as u64) / 2 {
        std::thread::sleep(Duration::from_millis(2));
    }
    p.cancel();
    assert!(matches!(h.join().unwrap(), Err(AiError::Cancelled)));
    let had = p.snapshot().bytes;
    assert!(had >= SIZE as u64 / 2 && had < SIZE as u64);

    // A new run (a new process in real life) fetches roughly the rest, and the final checksum covers it all.
    let second = Fake::new(data);
    let p = Progress::new();
    let path = download(&second, dir.path(), opts(), &p).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *second.data);
    let fetched = second.served.load(Ordering::SeqCst);
    assert!(
        fetched <= SIZE as u64 - had + 1024 * 1024,
        "fetched {fetched} after {had} were on disk"
    );
    assert!(!dir.path().join("big.gguf.part.ranges").exists());
}

#[test]
fn a_part_file_from_the_single_stream_downloader_resumes_in_segments() {
    let data = blob(SIZE);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("big.gguf.part"), &data[..2 << 20]).unwrap();
    let src = Fake::new(data);
    let p = Progress::new();
    let path = download(&src, dir.path(), opts(), &p).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *src.data);
    assert!(src.log.lock().unwrap().iter().all(|l| l.0 >= 2 << 20), "the first 2 MiB were kept");
}

#[test]
fn an_unreadable_sidecar_or_a_wrong_checksum_never_leaves_a_bad_file() {
    let data = blob(SIZE);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("big.gguf.part"), vec![7u8; SIZE]).unwrap();
    std::fs::write(dir.path().join("big.gguf.part.ranges"), "{ torn").unwrap();
    let src = Fake::new(data.clone());
    // Without a readable sidecar a part file of the full size cannot be told from a finished one: it is hashed,
    // found wrong and deleted (with its sidecar), and the next run starts clean.
    let r = download(&src, dir.path(), opts(), &Progress::new());
    assert!(matches!(r, Err(AiError::Integrity(_))), "{r:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let path = download(&src, dir.path(), opts(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), data);

    // The sidecar claims the whole file is there, but the bytes are wrong: the hash catches it and nothing is kept.
    let dir = tempfile::tempdir().unwrap();
    let remote = src.describe("o/r", "big.gguf").unwrap();
    std::fs::write(dir.path().join("big.gguf.part"), vec![7u8; SIZE]).unwrap();
    std::fs::write(
        dir.path().join("big.gguf.part.ranges"),
        json!({"version":1,"size":SIZE,"sha256":remote.sha256,"ranges":[[0,SIZE]]}).to_string(),
    )
    .unwrap();
    let r = Downloader {
        source: &src,
        disk: &Disk,
        opts: opts(),
    }
    .download(&remote, dir.path(), &Progress::new());
    assert!(matches!(r, Err(AiError::Integrity(_))), "{r:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // A host that serves other bytes than the catalogue promised.
    let dir = tempfile::tempdir().unwrap();
    let remote = RemoteFile {
        sha256: "0".repeat(64),
        ..src.describe("o/r", "big.gguf").unwrap()
    };
    let r = Downloader {
        source: &src,
        disk: &Disk,
        opts: opts(),
    }
    .download(&remote, dir.path(), &Progress::new());
    assert!(matches!(r, Err(AiError::Integrity(_))), "{r:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn progress_never_goes_backwards_or_past_the_total_and_names_the_connections() {
    let mut src = Fake::new(blob(SIZE));
    src.delay = Some(Box::new(|_, call| Duration::from_millis(if call % 3 == 0 { 3 } else { 1 })));
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Progress::new());
    let done = Arc::new(AtomicBool::new(false));
    let (p2, d2) = (p.clone(), done.clone());
    let watcher = std::thread::spawn(move || {
        let (mut last, mut peak, mut samples) = (0u64, 0u32, 0);
        while !d2.load(Ordering::Relaxed) {
            let s = p2.snapshot();
            assert!(s.bytes >= last && s.bytes <= SIZE as u64, "{} after {last}", s.bytes);
            assert_eq!(s.phase, Phase::Downloading);
            last = s.bytes;
            peak = peak.max(s.connections);
            samples += 1;
            std::thread::sleep(Duration::from_millis(3));
        }
        (peak, samples)
    });
    download(&src, dir.path(), opts(), &p).unwrap();
    done.store(true, Ordering::Relaxed);
    let (peak, samples) = watcher.join().unwrap();
    assert!(samples > 5 && peak >= 2, "{samples} samples, {peak} connections");
    let end = p.snapshot();
    assert_eq!((end.bytes, end.total), (SIZE as u64, SIZE as u64));
}

// ---- the Hugging Face adapter ----

const REPO: &str = "org/repo";
const FILE: &str = "big.gguf";

/// A mock Hugging Face whose CDN honours `Range` and limits every connection to `rate` bytes per second.
fn hf_server(data: Arc<Vec<u8>>, rate: Option<u64>, ranges: bool) -> Mock {
    let sha = sha256_hex(&data);
    Mock::start(move |req, port| {
        let p = req.path.as_str();
        if p == format!("/api/models/{REPO}") {
            Resp::json(200, json!({"id": REPO, "gated": false}))
        } else if p == format!("/api/models/{REPO}/paths-info/main") {
            Resp::json(
                200,
                json!([{"type":"file","path":FILE,"size":data.len(),"lfs":{"oid":sha,"size":data.len()}}]),
            )
        } else if p == format!("/{REPO}/resolve/main/{FILE}") {
            Resp::redirect(&format!("http://localhost:{port}/cdn/{FILE}"))
        } else if p.starts_with("/cdn/") {
            let mut r = if ranges {
                partial(req, &data)
            } else {
                Resp::new(200, data.to_vec())
            };
            r.rate_bps = rate;
            r
        } else {
            Resp::new(404, "no")
        }
    })
}

fn hf(m: &Mock, chunk: u64) -> HuggingFace {
    let mut cfg = HfConfig::local_test(&m.url());
    cfg.extra_hosts = vec!["localhost".into()];
    cfg.chunk_bytes = chunk;
    cfg.chunk_timeout = Duration::from_secs(20);
    HuggingFace::new(cfg).unwrap()
}

fn hf_download(src: &HuggingFace, connections: usize) -> (Duration, Vec<u8>) {
    let remote = src.describe(REPO, FILE).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts();
    o.connections = connections;
    o.min_block = 256 * 1024;
    let t = Instant::now();
    let path = Downloader {
        source: src,
        disk: &Disk,
        opts: o,
    }
    .download(&remote, dir.path(), &Progress::new())
    .unwrap();
    (t.elapsed(), std::fs::read(path).unwrap())
}

#[test]
fn the_adapter_serves_ranges_and_detects_hosts_that_do_not() {
    let data = Arc::new(blob(3 << 20));
    let m = hf_server(data.clone(), None, true);
    let src = hf(&m, 256 * 1024);
    let remote = src.describe(REPO, FILE).unwrap();
    assert!(src.supports_ranges(&remote));
    let mut got = Vec::new();
    let never = AtomicBool::new(false);
    src.fetch_range(
        &remote,
        1000,
        700_000,
        &mut |b| {
            got.extend_from_slice(b);
            Ok(())
        },
        &never,
    )
    .unwrap();
    assert_eq!(got, data[1000..700_000]);
    // Asking beyond the end is clipped to the file.
    let mut tail = 0;
    src.fetch_range(
        &remote,
        (3 << 20) - 10,
        99 << 20,
        &mut |b| {
            tail += b.len();
            Ok(())
        },
        &never,
    )
    .unwrap();
    assert_eq!(tail, 10);

    let plain = hf_server(data, None, false);
    let src = hf(&plain, 256 * 1024);
    let remote = src.describe(REPO, FILE).unwrap();
    assert!(!src.supports_ranges(&remote), "a 200 answer to a Range request is no range support");
}

#[test]
fn a_hugging_face_download_in_segments_matches_the_original() {
    let data = Arc::new(blob(5 << 20));
    let m = hf_server(data.clone(), None, true);
    let (_, got) = hf_download(&hf(&m, 512 * 1024), 4);
    assert_eq!(got, *data);
}

/// Per-connection limits are what segments beat: 4 connections move the same file about 3-4 times faster.
#[test]
fn segments_are_faster_than_one_connection_on_a_throttled_server() {
    let data = Arc::new(blob(4 << 20));
    let m = hf_server(data.clone(), Some(4 << 20), true);
    let src = hf(&m, 512 * 1024);
    let (one, a) = hf_download(&src, 1);
    let (four, b) = hf_download(&src, 4);
    assert_eq!((&a, &b), (&*data, &*data));
    eprintln!(
        "1 connection {one:?}, 4 connections {four:?}, speed-up x{:.1}",
        one.as_secs_f64() / four.as_secs_f64()
    );
    assert!(four.as_secs_f64() * 2.0 < one.as_secs_f64(), "{one:?} vs {four:?}");
}

/// `cargo test -p debrief-ai --test segmented_download --release -- --ignored --nocapture`
#[test]
#[ignore = "benchmark: prints the speed-up on a local server that limits each connection to 8 MiB/s"]
fn benchmark_connections_against_a_throttled_server() {
    let data = Arc::new(blob(64 << 20));
    let m = hf_server(data.clone(), Some(8 << 20), true);
    let src = hf(&m, 4 << 20);
    let base = hf_download(&src, 1).0;
    println!("64 MiB, 8 MiB/s per connection");
    println!("  1 connection : {base:>8.2?}  (x1.0)");
    for n in [2usize, 4, 8] {
        let t = hf_download(&src, n).0;
        println!("  {n} connections: {t:>8.2?}  (x{:.1})", base.as_secs_f64() / t.as_secs_f64());
    }
}
