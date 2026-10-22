//! What the host's memory watcher tells about a capped child (ADR 0031 §5): the pure [`MemoryWatch`] that turns
//! samples of the memory a child uses into [`MemoryEvent`]s (the peak, a warning at [`WARN_PERCENT`] of the cap with
//! hysteresis, the stop over the cap), and the [`MemoryListener`] a host passes in [`crate::process::SpawnSpec`] to
//! hear them. The sampling thread is in [`crate::process`].

use std::sync::Arc;

/// The share of the cap at which the host is warned.
pub const WARN_PERCENT: u64 = 80;
/// The share of the cap below which a warning is armed again (so a child hovering at 80% warns once).
pub const REARM_PERCENT: u64 = 70;

/// A child's memory at one moment: what it uses, its peak so far and its cap (bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryUse {
    pub used: u64,
    pub peak: u64,
    pub limit: u64,
}

/// What the watcher reports about one child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEvent {
    /// It crossed [`WARN_PERCENT`] of its cap (again after falling below [`REARM_PERCENT`]).
    Warning(MemoryUse),
    /// It used more than its cap and the host stopped it.
    Stopped(MemoryUse),
    /// It ended on its own (or was stopped by IMT for another reason); `peak` is the most it used.
    Ended(MemoryUse),
}

impl MemoryEvent {
    pub fn usage(&self) -> MemoryUse {
        match self {
            MemoryEvent::Warning(u) | MemoryEvent::Stopped(u) | MemoryEvent::Ended(u) => *u,
        }
    }
}

/// Hears the events of one child: called on the watcher's thread with the child's label (`plugin "x"`, `the store
/// worker`) and the event. Must return quickly and must not panic (a panic is caught and ignored).
#[derive(Clone)]
pub struct MemoryListener(Arc<ListenFn>);

type ListenFn = dyn Fn(&str, MemoryEvent) + Send + Sync;

impl MemoryListener {
    pub fn new(f: impl Fn(&str, MemoryEvent) + Send + Sync + 'static) -> MemoryListener {
        MemoryListener(Arc::new(f))
    }

    /// Tells the listener; a panicking listener never takes the watcher down.
    pub fn tell(&self, label: &str, event: MemoryEvent) {
        let f = self.0.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(label, event)));
    }
}

impl std::fmt::Debug for MemoryListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryListener")
    }
}

/// The state of watching one child against its cap. Pure: [`MemoryWatch::observe`] takes a sample and answers what to
/// report, so every threshold is unit-tested without processes or clocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWatch {
    limit: u64,
    peak: u64,
    armed: bool,
}

/// What one sample means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to report.
    Fine,
    /// Report a warning; the child keeps running.
    Warn(MemoryUse),
    /// Over the cap: stop the child and report it.
    Stop(MemoryUse),
}

impl MemoryWatch {
    pub fn new(limit: u64) -> MemoryWatch {
        MemoryWatch {
            limit,
            peak: 0,
            armed: true,
        }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn peak(&self) -> u64 {
        self.peak
    }

    fn usage(&self, used: u64) -> MemoryUse {
        MemoryUse {
            used,
            peak: self.peak,
            limit: self.limit,
        }
    }

    /// The level of the warning, in bytes.
    pub fn warn_at(&self) -> u64 {
        share(self.limit, WARN_PERCENT)
    }

    /// Takes a sample of the memory the child uses now.
    pub fn observe(&mut self, used: u64) -> Verdict {
        self.peak = self.peak.max(used);
        if used > self.limit {
            return Verdict::Stop(self.usage(used));
        }
        if used >= self.warn_at() {
            if self.armed {
                self.armed = false;
                return Verdict::Warn(self.usage(used));
            }
        } else if used < share(self.limit, REARM_PERCENT) {
            self.armed = true;
        }
        Verdict::Fine
    }

    /// The report when the child ended without being stopped over its cap.
    pub fn ended(&self, used: u64) -> MemoryUse {
        self.usage(used)
    }
}

fn share(limit: u64, percent: u64) -> u64 {
    // In u128: a cap near u64::MAX must not overflow.
    (u128::from(limit) * u128::from(percent) / 100) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1 << 20;

    #[test]
    fn it_warns_once_at_80_percent_until_the_use_falls_below_70() {
        let mut w = MemoryWatch::new(500 * MB);
        assert_eq!(w.warn_at(), 400 * MB);
        assert_eq!(w.observe(100 * MB), Verdict::Fine);
        assert_eq!(w.observe(399 * MB), Verdict::Fine);
        let warn = w.observe(420 * MB);
        assert_eq!(
            warn,
            Verdict::Warn(MemoryUse {
                used: 420 * MB,
                peak: 420 * MB,
                limit: 500 * MB
            })
        );
        // Hovering around the level does not warn again.
        assert_eq!(w.observe(450 * MB), Verdict::Fine);
        assert_eq!(w.observe(360 * MB), Verdict::Fine, "75% is not low enough to re-arm");
        assert_eq!(w.observe(410 * MB), Verdict::Fine);
        // Below 70% it is armed again.
        assert_eq!(w.observe(300 * MB), Verdict::Fine);
        assert!(matches!(w.observe(400 * MB), Verdict::Warn(u) if u.peak == 450 * MB));
    }

    #[test]
    fn over_the_cap_it_stops_with_the_peak() {
        let mut w = MemoryWatch::new(256 * MB);
        assert_eq!(
            w.observe(256 * MB),
            Verdict::Warn(w.usage(256 * MB)),
            "at the cap: warned, not stopped"
        );
        let stop = w.observe(257 * MB);
        assert_eq!(
            stop,
            Verdict::Stop(MemoryUse {
                used: 257 * MB,
                peak: 257 * MB,
                limit: 256 * MB
            })
        );
        // A jump straight over the cap stops without a warning first.
        let mut jump = MemoryWatch::new(256 * MB);
        assert!(matches!(jump.observe(900 * MB), Verdict::Stop(u) if u.peak == 900 * MB));
    }

    #[test]
    fn the_peak_is_kept_for_the_end_report_and_huge_caps_do_not_overflow() {
        let mut w = MemoryWatch::new(u64::MAX);
        assert_eq!(w.observe(5), Verdict::Fine);
        assert_eq!(w.observe(3), Verdict::Fine);
        assert_eq!(
            w.ended(3),
            MemoryUse {
                used: 3,
                peak: 5,
                limit: u64::MAX
            }
        );
        assert_eq!(w.peak(), 5);
        assert!(w.warn_at() > u64::MAX / 2);
        let mut zero = MemoryWatch::new(0);
        assert!(matches!(zero.observe(1), Verdict::Stop(_)));
    }

    #[test]
    fn a_panicking_listener_is_contained() {
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let h = heard.clone();
        let ok = MemoryListener::new(move |label, e| h.lock().unwrap().push((label.to_string(), e)));
        let u = MemoryUse {
            used: 1,
            peak: 1,
            limit: 2,
        };
        ok.tell("plugin \"x\"", MemoryEvent::Warning(u));
        assert_eq!(heard.lock().unwrap()[0], ("plugin \"x\"".to_string(), MemoryEvent::Warning(u)));
        let bad = MemoryListener::new(|_, _| panic!("listener bug"));
        bad.tell("x", MemoryEvent::Ended(u));
        assert_eq!(MemoryEvent::Stopped(u).usage(), u);
        assert_eq!(format!("{ok:?}"), "MemoryListener");
    }
}
