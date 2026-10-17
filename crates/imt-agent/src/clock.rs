//! Time as a dependency, so schedules, triggers and the service loop are
//! testable with a fixed clock.

use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Contract: `now` is the wall clock; `sleep` waits up to `d` and returns
/// early once `stop` is set.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    fn sleep(&self, d: Duration, stop: &AtomicBool);
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep(&self, d: Duration, stop: &AtomicBool) {
        let end = Instant::now() + d;
        while !stop.load(Ordering::Relaxed) {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
    }
}

/// Milliseconds since the Unix epoch.
pub fn ms(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_sleeps_and_stops_early() {
        let c = SystemClock;
        let before = c.now();
        let start = Instant::now();
        c.sleep(Duration::from_millis(30), &AtomicBool::new(false));
        assert!(start.elapsed() >= Duration::from_millis(30));
        let start = Instant::now();
        c.sleep(Duration::from_secs(5), &AtomicBool::new(true));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(ms(c.now()) >= ms(before));
    }
}
