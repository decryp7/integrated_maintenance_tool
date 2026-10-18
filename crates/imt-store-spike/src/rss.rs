//! Resident-set sampling: a background thread polls the process RSS, keeps the peak of the current
//! phase and aborts the run when it passes a limit, so a benchmark cannot exhaust a shared machine.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct Sampler {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

/// Current resident set size in bytes (0 when the platform cannot tell).
pub fn current() -> u64 {
    memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
}

impl Sampler {
    /// Starts sampling every 20 ms; exits the process with code 3 above `limit` bytes.
    pub fn start(limit: u64) -> Sampler {
        let peak = Arc::new(AtomicU64::new(current()));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                let now = current();
                p.fetch_max(now, Ordering::Relaxed);
                if limit > 0 && now > limit {
                    eprintln!("RSS {} MB passed the limit of {} MB; aborting", now >> 20, limit >> 20);
                    std::process::exit(3);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Sampler { peak, stop }
    }

    /// Peak RSS since the last reset, in MB, and starts a new phase.
    pub fn take_peak_mb(&self) -> u64 {
        let now = current();
        let peak = self.peak.swap(now, Ordering::Relaxed).max(now);
        peak >> 20
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
