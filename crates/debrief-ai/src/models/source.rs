//! The port for fetching model files, plus progress reporting shared by every stage.

use crate::error::Result;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What the model host says about one file.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFile {
    pub repo: String,
    pub file: String,
    pub size: u64,
    /// Lower-case hex SHA-256 (the LFS object id).
    pub sha256: String,
    pub license: Option<String>,
    /// The repository requires accepting terms on the host.
    pub gated: bool,
}

/// Contract: `describe` never downloads file contents; `fetch` streams the bytes from `offset` to the
/// end of the file through `sink`, honours `cancel` between buffers (returning `AiError::Cancelled`),
/// and never writes anywhere itself.
///
/// Segmented (multi-connection) downloads need two more capabilities, both optional: a source that cannot serve
/// byte ranges keeps the defaults and is downloaded as one stream.
/// * `supports_ranges` may make one small request; it answers `true` only when the host serves `Range` requests
///   for this file (206 with a matching `Content-Range`) and never fails: any doubt is `false`.
/// * `fetch_range` streams exactly the bytes `start..end` (end exclusive) through `sink`, on its own connection,
///   under the same rules as `fetch` (redirects, allowed hosts, timeouts, `cancel`). It is called from several
///   threads at once. A `sink` error stops it and is returned as is.
pub trait ModelSource: Send + Sync {
    fn describe(&self, repo: &str, file: &str) -> Result<RemoteFile>;
    fn fetch(&self, remote: &RemoteFile, offset: u64, sink: &mut dyn FnMut(&[u8]) -> Result<()>, cancel: &AtomicBool) -> Result<()>;

    fn supports_ranges(&self, _remote: &RemoteFile) -> bool {
        false
    }

    fn fetch_range(
        &self,
        _remote: &RemoteFile,
        _start: u64,
        _end: u64,
        _sink: &mut dyn FnMut(&[u8]) -> Result<()>,
        _cancel: &AtomicBool,
    ) -> Result<()> {
        Err(crate::error::AiError::Refused("this source cannot download byte ranges".into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    Verifying,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressSnapshot {
    pub phase: Phase,
    pub bytes: u64,
    pub total: u64,
    pub bytes_per_sec: f64,
    pub eta_secs: Option<u64>,
    /// Connections in use right now (1 for a single stream).
    pub connections: u32,
}

/// Lock-free progress and cancellation, readable from any thread (like the importer's progress).
pub struct Progress {
    bytes: AtomicU64,
    total: AtomicU64,
    phase: AtomicU8,
    /// Bytes that were already on disk when this phase began (resume).
    base: AtomicU64,
    started: Mutex<Instant>,
    connections: AtomicU32,
    /// Smoothed speed: (when sampled, bytes then, exponential average in bytes/s).
    rate: Mutex<(Instant, u64, f64)>,
    cancel: Arc<AtomicBool>,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        Self::with_cancel(Arc::new(AtomicBool::new(false)))
    }

    pub fn with_cancel(cancel: Arc<AtomicBool>) -> Self {
        Progress {
            bytes: AtomicU64::new(0),
            total: AtomicU64::new(0),
            phase: AtomicU8::new(0),
            base: AtomicU64::new(0),
            started: Mutex::new(Instant::now()),
            connections: AtomicU32::new(1),
            rate: Mutex::new((Instant::now(), 0, 0.0)),
            cancel,
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn begin(&self, phase: Phase, total: u64, already: u64) {
        self.phase.store(phase as u8, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.bytes.store(already, Ordering::Relaxed);
        self.base.store(already, Ordering::Relaxed);
        if let Ok(mut s) = self.started.lock() {
            *s = Instant::now();
        }
        self.connections.store(1, Ordering::Relaxed);
        if let Ok(mut r) = self.rate.lock() {
            *r = (Instant::now(), already, 0.0);
        }
    }

    /// Connections currently downloading (shown in the detail of the job).
    pub fn set_connections(&self, n: u32) {
        self.connections.store(n, Ordering::Relaxed);
    }

    pub fn add(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        let bytes = self.bytes.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let elapsed = self.started.lock().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        let done = bytes.saturating_sub(self.base.load(Ordering::Relaxed)) as f64;
        let average = if elapsed > 0.05 { done / elapsed } else { 0.0 };
        // Smoothed: an exponential average over samples at least 250 ms apart, so the speed (and the ETA made
        // from it) does not jump with every poll or when segments start and finish.
        let rate = match self.rate.lock() {
            Ok(mut r) => {
                let dt = r.0.elapsed().as_secs_f64();
                if dt >= 0.25 {
                    let inst = bytes.saturating_sub(r.1) as f64 / dt;
                    r.2 = if r.2 > 0.0 { 0.3 * inst + 0.7 * r.2 } else { inst };
                    *r = (Instant::now(), bytes, r.2);
                }
                if r.2 > 0.0 {
                    r.2
                } else {
                    average
                }
            }
            Err(_) => average,
        };
        let eta = (rate > 1.0 && total >= bytes).then(|| ((total - bytes) as f64 / rate).ceil() as u64);
        let phase = if self.phase.load(Ordering::Relaxed) == 1 {
            Phase::Verifying
        } else {
            Phase::Downloading
        };
        ProgressSnapshot {
            phase,
            bytes,
            total,
            bytes_per_sec: rate,
            eta_secs: eta,
            connections: self.connections.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_tracks_bytes_rate_and_cancel() {
        let p = Progress::new();
        p.begin(Phase::Downloading, 1000, 100);
        std::thread::sleep(std::time::Duration::from_millis(80));
        p.add(400);
        let s = p.snapshot();
        assert_eq!((s.bytes, s.total, s.phase), (500, 1000, Phase::Downloading));
        assert!(s.bytes_per_sec > 0.0);
        assert!(s.eta_secs.is_some());
        p.begin(Phase::Verifying, 10, 0);
        assert_eq!(p.snapshot().phase, Phase::Verifying);
        assert!(!p.is_cancelled());
        p.cancel();
        assert!(p.is_cancelled() && p.cancel_flag().load(Ordering::Relaxed));
        assert_eq!(Progress::default().snapshot().bytes, 0);
    }
}
