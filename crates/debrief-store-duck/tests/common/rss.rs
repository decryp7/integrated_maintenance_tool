//! Resident-set sampling (from the store spike's `rss.rs`): a thread polls the process RSS every 20 ms and
//! keeps the peak of the current phase.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Current resident set size in bytes (0 when the platform cannot tell).
pub fn current() -> u64 {
    memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
}

pub struct Sampler {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Sampler {
    pub fn start() -> Sampler {
        let peak = Arc::new(AtomicU64::new(current()));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(current(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Sampler { peak, stop }
    }

    /// Peak RSS since the start or the last call, in MB, and starts a new phase.
    pub fn take_peak_mb(&self) -> u64 {
        let now = current();
        self.peak.swap(now, Ordering::Relaxed).max(now) >> 20
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
