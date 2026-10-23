//! Segmented (multi-connection) downloads of one file.
//!
//! One connection to a CDN is often limited per connection, not per client, so several `Range` requests in
//! parallel are several times faster for large files. This module is the engine for that; [`super::download`]
//! decides when to use it and verifies the result.
//!
//! * The file is preallocated and **positioned writes** put each buffer at its own offset (no shared cursor).
//! * Work is a queue of blocks (about `size / (8 x connections)`, 1 to 16 MiB). A worker takes the next block;
//!   when the queue is empty, an idle worker **steals** the far half of the largest unfinished block, so the tail
//!   never waits on one slow connection.
//! * A failing block retries on its own remaining range with exponential backoff. A 429/503 lowers the number of
//!   connections (down to one) and honours `Retry-After`.
//! * **No second pass.** A hasher thread reads the contiguous prefix that is complete (just written, so still in
//!   the page cache) in file order and feeds SHA-256 while the download continues. When the last byte lands, the
//!   digest is almost ready.
//! * **Resume.** Completed ranges are written to a small versioned sidecar (`<file>.part.ranges`) about once a
//!   second and on every stop. A resumed download fetches only the missing ranges; the hasher re-reads the
//!   already downloaded prefix from disk once (local read speed, overlapped with the network transfer), which is
//!   the price of not trusting unverified bytes.
//! * Memory is bounded: one 64 KiB buffer per connection plus a 1 MiB hash buffer. Cancelling is noticed after
//!   every buffer (a connection stalled inside a read ends when its read times out).

use super::download::io_err;
use super::source::{ModelSource, Phase, Progress, RemoteFile};
use crate::error::{AiError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Most connections one download may use, whatever the setting says.
pub const MAX_CONNECTIONS: usize = 8;
const MAX_BLOCK: u64 = 16 * 1024 * 1024;
const SIDECAR_VERSION: u32 = 1;
const SIDECAR_MAX_BYTES: u64 = 1024 * 1024;
const MAX_THROTTLE_WAIT: Duration = Duration::from_secs(30);
const MAX_THROTTLES: u32 = 24;

/// Caps the connections open to the model hosts across every download of the process.
#[derive(Debug)]
pub struct HostGate {
    cap: usize,
    in_use: AtomicUsize,
}

impl HostGate {
    pub fn new(cap: usize) -> Arc<HostGate> {
        Arc::new(HostGate {
            cap: cap.max(1),
            in_use: AtomicUsize::new(0),
        })
    }

    /// The gate every download of the app shares (at most [`MAX_CONNECTIONS`] open connections in total).
    pub fn shared() -> Arc<HostGate> {
        static GATE: OnceLock<Arc<HostGate>> = OnceLock::new();
        GATE.get_or_init(|| HostGate::new(MAX_CONNECTIONS)).clone()
    }

    fn try_acquire(self: &Arc<Self>) -> Option<GateGuard> {
        let mut cur = self.in_use.load(Ordering::Relaxed);
        loop {
            if cur >= self.cap {
                return None;
            }
            match self.in_use.compare_exchange(cur, cur + 1, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return Some(GateGuard(self.clone())),
                Err(now) => cur = now,
            }
        }
    }
}

struct GateGuard(Arc<HostGate>);

impl Drop for GateGuard {
    fn drop(&mut self) {
        self.0.in_use.fetch_sub(1, Ordering::AcqRel);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(unix)]
fn write_all_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, off)
}

#[cfg(unix)]
fn read_exact_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, off)
}

#[cfg(windows)]
fn write_all_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = f.seek_write(&buf[done..], off + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        done += n;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = f.seek_read(&mut buf[done..], off + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

/// The byte ranges that are on disk, kept sorted and merged.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Coverage {
    map: BTreeMap<u64, u64>,
    covered: u64,
}

impl Coverage {
    pub fn prefix_of(len: u64) -> Coverage {
        let mut c = Coverage::default();
        c.add(0, len);
        c
    }

    /// Adds `start..end` and returns how many of those bytes were new.
    pub fn add(&mut self, start: u64, end: u64) -> u64 {
        if start >= end {
            return 0;
        }
        let from = self
            .map
            .range(..=start)
            .next_back()
            .filter(|(_, &b)| b >= start)
            .map_or(start, |(&a, _)| a);
        let keys: Vec<u64> = self.map.range(from..=end).map(|(&a, _)| a).collect();
        let (mut lo, mut hi, mut overlap) = (start, end, 0u64);
        for a in keys {
            if let Some(b) = self.map.remove(&a) {
                overlap += b.min(end).saturating_sub(a.max(start));
                self.covered -= b - a;
                lo = lo.min(a);
                hi = hi.max(b);
            }
        }
        self.map.insert(lo, hi);
        self.covered += hi - lo;
        (end - start) - overlap
    }

    pub fn covered(&self) -> u64 {
        self.covered
    }

    /// Length of the contiguous run that starts at byte 0.
    pub fn prefix(&self) -> u64 {
        self.map.get(&0).copied().unwrap_or(0)
    }

    pub fn ranges(&self) -> Vec<(u64, u64)> {
        self.map.iter().map(|(&a, &b)| (a, b)).collect()
    }

    /// The gaps inside `0..size`.
    pub fn missing(&self, size: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut at = 0;
        for (&a, &b) in &self.map {
            if a > at {
                out.push((at, a.min(size)));
            }
            at = at.max(b);
        }
        if at < size {
            out.push((at, size));
        }
        out
    }
}

/// What a resumable segmented download remembers between runs.
#[derive(Serialize, Deserialize)]
struct SidecarFile {
    version: u32,
    size: u64,
    /// The expected SHA-256 doubles as the validator: another file under the same name never resumes.
    sha256: String,
    ranges: Vec<(u64, u64)>,
}

pub(super) fn sidecar_path(part: &Path) -> std::path::PathBuf {
    let mut s = part.as_os_str().to_owned();
    s.push(".ranges");
    s.into()
}

/// The ranges saved for `remote`, or `None` when there is no usable sidecar (missing, corrupt, another version,
/// another file, or the `.part` file is not the preallocated size).
pub(super) fn load_sidecar(path: &Path, remote: &RemoteFile, part_len: u64) -> Option<Coverage> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > SIDECAR_MAX_BYTES || part_len != remote.size {
        return None;
    }
    let f: SidecarFile = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if f.version != SIDECAR_VERSION || f.size != remote.size || f.sha256 != remote.sha256 {
        return None;
    }
    let mut c = Coverage::default();
    for (a, b) in f.ranges {
        if a >= b || b > remote.size {
            return None;
        }
        c.add(a, b);
    }
    Some(c)
}

fn save_sidecar(path: &Path, remote: &RemoteFile, cov: &Coverage) -> std::io::Result<()> {
    let body = serde_json::to_vec(&SidecarFile {
        version: SIDECAR_VERSION,
        size: remote.size,
        sha256: remote.sha256.clone(),
        ranges: cov.ranges(),
    })
    .map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// How a segmented run is tuned (a slice of [`super::DownloadOptions`]).
#[derive(Clone)]
pub(super) struct Tuning {
    pub connections: usize,
    pub min_block: u64,
    pub attempts: u32,
    pub backoff: Duration,
    pub gate: Option<Arc<HostGate>>,
}

/// A byte range being downloaded. `end` shrinks when an idle worker steals the far half.
struct Task {
    pos: AtomicU64,
    end: AtomicU64,
}

impl Task {
    fn remaining(&self) -> u64 {
        self.end.load(Ordering::Acquire).saturating_sub(self.pos.load(Ordering::Acquire))
    }
}

#[derive(Default)]
struct Board {
    pending: VecDeque<(u64, u64)>,
    running: Vec<Arc<Task>>,
}

enum Next {
    Task(Arc<Task>),
    Wait,
    Done,
}

struct Shared<'a> {
    source: &'a dyn ModelSource,
    remote: &'a RemoteFile,
    tuning: &'a Tuning,
    progress: &'a Progress,
    cancel: Arc<AtomicBool>,
    file: File,
    cov: Mutex<Coverage>,
    board: Mutex<Board>,
    /// Workers with an id at or above this stay idle (lowered by 429/503).
    limit: AtomicUsize,
    active: AtomicUsize,
    stop: AtomicBool,
    fatal: Mutex<Option<AiError>>,
}

impl Shared<'_> {
    fn halt(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed)
    }

    fn fail(&self, e: AiError) {
        lock(&self.fatal).get_or_insert(e);
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Sleeps `d` in short slices so a cancel is noticed quickly. Returns false when the download is halting.
    fn nap(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if self.halt() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10).min(until.saturating_duration_since(Instant::now())));
        }
        !self.halt()
    }

    fn next_task(&self) -> Next {
        let mut b = lock(&self.board);
        if let Some((s, e)) = b.pending.pop_front() {
            let t = Arc::new(Task {
                pos: AtomicU64::new(s),
                end: AtomicU64::new(e),
            });
            b.running.push(t.clone());
            return Next::Task(t);
        }
        let victim = b.running.iter().max_by_key(|t| t.remaining()).cloned();
        if let Some(v) = victim {
            let rem = v.remaining();
            if rem >= 2 * self.tuning.min_block {
                let end = v.end.load(Ordering::Acquire);
                let mid = v.pos.load(Ordering::Acquire) + rem / 2;
                v.end.store(mid, Ordering::Release);
                let t = Arc::new(Task {
                    pos: AtomicU64::new(mid),
                    end: AtomicU64::new(end),
                });
                b.running.push(t.clone());
                return Next::Task(t);
            }
        }
        if b.running.is_empty() {
            Next::Done
        } else {
            Next::Wait
        }
    }

    fn finish(&self, t: &Arc<Task>) {
        lock(&self.board).running.retain(|r| !Arc::ptr_eq(r, t));
    }

    /// Gives the unfinished rest of `t` back to the queue (a worker that must stand down).
    fn requeue(&self, t: &Arc<Task>) {
        let mut b = lock(&self.board);
        b.running.retain(|r| !Arc::ptr_eq(r, t));
        let (p, e) = (t.pos.load(Ordering::Acquire), t.end.load(Ordering::Acquire));
        if p < e {
            b.pending.push_front((p, e));
        }
    }

    fn set_active(&self, delta: isize) {
        let n = if delta > 0 {
            self.active.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            self.active.fetch_sub(1, Ordering::AcqRel) - 1
        };
        self.progress.set_connections(n.max(1) as u32);
    }

    fn lower_limit(&self) {
        let mut l = self.limit.load(Ordering::Acquire);
        while l > 1 {
            match self.limit.compare_exchange(l, l - 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => l = now,
            }
        }
    }

    fn worker(&self, id: usize) {
        loop {
            if self.halt() {
                return;
            }
            if id >= self.limit.load(Ordering::Acquire) {
                if lock(&self.board).running.is_empty() && lock(&self.board).pending.is_empty() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            match self.next_task() {
                Next::Task(t) => self.run_task(id, t),
                Next::Wait => std::thread::sleep(Duration::from_millis(10)),
                Next::Done => return,
            }
        }
    }

    fn run_task(&self, id: usize, t: Arc<Task>) {
        let (mut failures, mut throttles) = (0u32, 0u32);
        loop {
            if self.halt() {
                return;
            }
            let (pos, end) = (t.pos.load(Ordering::Acquire), t.end.load(Ordering::Acquire));
            if pos >= end {
                return self.finish(&t);
            }
            let _slot = match &self.tuning.gate {
                Some(g) => loop {
                    if let Some(s) = g.try_acquire() {
                        break Some(s);
                    }
                    if !self.nap(Duration::from_millis(10)) {
                        return;
                    }
                },
                None => None,
            };
            self.set_active(1);
            let mut sink = |b: &[u8]| -> Result<()> {
                if self.halt() {
                    return Err(AiError::Cancelled);
                }
                let pos = t.pos.load(Ordering::Acquire);
                let end = t.end.load(Ordering::Acquire);
                let take = end.saturating_sub(pos).min(b.len() as u64) as usize;
                if take > 0 {
                    write_all_at(&self.file, &b[..take], pos).map_err(io_err)?;
                    let fresh = lock(&self.cov).add(pos, pos + take as u64);
                    self.progress.add(fresh);
                    t.pos.store(pos + take as u64, Ordering::Release);
                }
                // Reached the (possibly shrunk) end of this task: stop the request, it is complete.
                if take < b.len() || pos + take as u64 >= end {
                    return Err(AiError::Cancelled);
                }
                Ok(())
            };
            let result = self.source.fetch_range(self.remote, pos, end, &mut sink, &self.cancel);
            self.set_active(-1);
            drop(_slot);
            if t.pos.load(Ordering::Acquire) >= t.end.load(Ordering::Acquire) {
                return self.finish(&t);
            }
            let made_progress = t.pos.load(Ordering::Acquire) > pos;
            let err = match result {
                Ok(()) => AiError::Network("the connection closed early".into()),
                Err(AiError::Cancelled) => {
                    if self.halt() {
                        return;
                    }
                    // The task was shrunk by a steal while the request ran; ask again for the rest.
                    continue;
                }
                Err(e) => e,
            };
            if !err.is_transient() {
                self.fail(err);
                return;
            }
            if made_progress {
                failures = 0;
            }
            if let Some(after) = err.throttle() {
                // Giving ground (fewer connections) is not a failure; being refused on the last one is.
                if self.limit.load(Ordering::Acquire) <= 1 {
                    throttles += 1;
                    if throttles > MAX_THROTTLES {
                        self.fail(err);
                        return;
                    }
                }
                self.lower_limit();
                let wait = after.unwrap_or(self.tuning.backoff).max(self.tuning.backoff).min(MAX_THROTTLE_WAIT);
                if !self.nap(wait) {
                    return;
                }
                if id >= self.limit.load(Ordering::Acquire) {
                    return self.requeue(&t);
                }
                continue;
            }
            failures += 1;
            if failures >= self.tuning.attempts {
                self.fail(err);
                return;
            }
            if !self.nap(self.tuning.backoff * 2u32.saturating_pow(failures - 1)) {
                return;
            }
        }
    }

    /// Feeds the file to SHA-256 in order, as far as the contiguous prefix has been written.
    fn hasher(&self) -> Result<String> {
        let size = self.remote.size;
        let (mut h, mut done) = (Sha256::new(), 0u64);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if done >= size {
                return Ok(super::download::hex(&h.finalize()));
            }
            let prefix = lock(&self.cov).prefix();
            if done < prefix {
                let n = (prefix - done).min(buf.len() as u64) as usize;
                read_exact_at(&self.file, &mut buf[..n], done).map_err(io_err)?;
                h.update(&buf[..n]);
                done += n as u64;
            } else if self.halt() {
                return Err(AiError::Cancelled);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn block_size(size: u64, connections: usize, min_block: u64) -> u64 {
    let b = size / (connections as u64 * 8);
    b.min(MAX_BLOCK).max(min_block).max(1)
}

/// Downloads the ranges of `remote` that `have` does not cover into `part` (preallocated to the full size) with
/// parallel connections and returns the SHA-256 of the whole file. The sidecar keeps the progress when this
/// fails or is cancelled; the caller deletes it once the file is verified.
pub(super) fn run(
    source: &dyn ModelSource,
    remote: &RemoteFile,
    part: &Path,
    have: Coverage,
    tuning: &Tuning,
    progress: &Progress,
) -> Result<String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(part)
        .map_err(io_err)?;
    file.set_len(remote.size).map_err(io_err)?;
    let n = tuning.connections.clamp(1, MAX_CONNECTIONS);
    let block = block_size(remote.size, n, tuning.min_block);
    let mut board = Board::default();
    for (s, e) in have.missing(remote.size) {
        let mut at = s;
        while at < e {
            let next = (at + block).min(e);
            board.pending.push_back((at, next));
            at = next;
        }
    }
    let sidecar = sidecar_path(part);
    progress.begin(Phase::Downloading, remote.size, have.covered());
    let sh = Shared {
        source,
        remote,
        tuning,
        progress,
        cancel: progress.cancel_flag(),
        file,
        cov: Mutex::new(have),
        board: Mutex::new(board),
        limit: AtomicUsize::new(n),
        active: AtomicUsize::new(0),
        stop: AtomicBool::new(false),
        fatal: Mutex::new(None),
    };
    save_sidecar(&sidecar, remote, &lock(&sh.cov)).map_err(io_err)?;
    let alive = AtomicUsize::new(n);
    let digest = std::thread::scope(|s| {
        for id in 0..n {
            let (sh, alive) = (&sh, &alive);
            s.spawn(move || {
                sh.worker(id);
                alive.fetch_sub(1, Ordering::AcqRel);
            });
        }
        let hasher = s.spawn(|| sh.hasher());
        let mut saved = Instant::now();
        while alive.load(Ordering::Acquire) > 0 {
            std::thread::sleep(Duration::from_millis(10));
            if saved.elapsed() >= Duration::from_secs(1) {
                let _ = save_sidecar(&sidecar, remote, &lock(&sh.cov));
                saved = Instant::now();
            }
        }
        // Workers are done: wake the hasher if it is waiting for bytes that will never come.
        let complete = lock(&sh.cov).covered() >= remote.size;
        if !complete {
            sh.stop.store(true, Ordering::Relaxed);
        }
        hasher.join()
    });
    let _ = save_sidecar(&sidecar, remote, &lock(&sh.cov));
    if let Some(e) = lock(&sh.fatal).take() {
        return Err(e);
    }
    if sh.cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    if lock(&sh.cov).covered() < remote.size {
        return Err(AiError::Network(
            "the connections ended before the download finished; run it again to resume".into(),
        ));
    }
    digest.map_err(|_| AiError::Engine("the checksum thread stopped unexpectedly".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_merges_counts_new_bytes_and_finds_gaps() {
        let mut c = Coverage::default();
        assert_eq!(c.add(10, 20), 10);
        assert_eq!(c.add(30, 40), 10);
        assert_eq!(c.add(15, 35), 10, "only 20..30 is new");
        assert_eq!(c.ranges(), vec![(10, 40)]);
        assert_eq!(c.add(0, 10), 10);
        assert_eq!((c.prefix(), c.covered()), (40, 40));
        assert_eq!(c.add(5, 5), 0);
        assert_eq!(c.add(0, 40), 0);
        assert_eq!(c.missing(100), vec![(40, 100)]);
        let mut g = Coverage::default();
        g.add(20, 30);
        assert_eq!((g.prefix(), g.missing(50)), (0, vec![(0, 20), (30, 50)]));
        assert_eq!(Coverage::prefix_of(7).ranges(), vec![(0, 7)]);
    }

    fn remote(size: u64) -> RemoteFile {
        RemoteFile {
            repo: "o/r".into(),
            file: "f.bin".into(),
            size,
            sha256: "a".repeat(64),
            license: None,
            gated: false,
        }
    }

    #[test]
    fn sidecar_round_trips_and_rejects_anything_that_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar_path(&dir.path().join("f.bin.part"));
        let mut c = Coverage::default();
        c.add(0, 10);
        c.add(50, 60);
        save_sidecar(&path, &remote(100), &c).unwrap();
        assert_eq!(load_sidecar(&path, &remote(100), 100), Some(c));
        // Another size, another checksum, a part file of the wrong length, a missing file: no resume.
        assert!(load_sidecar(&path, &remote(101), 101).is_none());
        let mut other = remote(100);
        other.sha256 = "b".repeat(64);
        assert!(load_sidecar(&path, &other, 100).is_none());
        assert!(load_sidecar(&path, &remote(100), 99).is_none());
        assert!(load_sidecar(&dir.path().join("none"), &remote(100), 100).is_none());
        // Corrupt, wrong version and out-of-range data.
        for body in [
            "{not json".to_string(),
            r#"{"version":2,"size":100,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","ranges":[]}"#
                .to_string(),
            format!(r#"{{"version":1,"size":100,"sha256":"{}","ranges":[[0,200]]}}"#, "a".repeat(64)),
            format!(r#"{{"version":1,"size":100,"sha256":"{}","ranges":[[5,5]]}}"#, "a".repeat(64)),
        ] {
            std::fs::write(&path, body).unwrap();
            assert!(load_sidecar(&path, &remote(100), 100).is_none());
        }
    }

    #[test]
    fn gate_caps_open_connections() {
        let g = HostGate::new(2);
        let a = g.try_acquire();
        let b = g.try_acquire();
        assert!(a.is_some() && b.is_some() && g.try_acquire().is_none());
        drop(a);
        assert!(g.try_acquire().is_some());
        assert_eq!(HostGate::new(0).cap, 1);
        assert_eq!(HostGate::shared().cap, MAX_CONNECTIONS);
    }

    #[test]
    fn blocks_are_between_the_minimum_and_16_mib() {
        assert_eq!(block_size(10 << 30, 4, 1 << 20), MAX_BLOCK);
        assert_eq!(block_size(64 << 20, 4, 1 << 20), 2 << 20);
        assert_eq!(block_size(100, 4, 1 << 20), 1 << 20);
    }
}
