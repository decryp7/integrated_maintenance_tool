//! Keeps a child process healthy. The child is started lazily on first use; a call that crashes, hangs
//! or garbles the protocol kills it, fails with a clear error, and the next call starts a fresh one after
//! an exponential backoff. After [`SupervisorConfig::max_crashes`] failures within
//! [`SupervisorConfig::crash_window`] the child is quarantined: no more restarts until [`Supervisor::reset`].
//!
//! [`CrashLedger`] (the failure history and quarantine) and [`backoff_delay`] are usable on their own, for
//! supervisors with richer call sequencing (the AI engine reloads its model after a restart).

use crate::IpcError;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Failure history of a child (across restarts) and its quarantine.
pub struct CrashLedger {
    max: usize,
    window: Duration,
    crashes: Mutex<VecDeque<Instant>>,
    disabled: Mutex<Option<String>>,
    last: Mutex<Option<String>>,
    restarts: AtomicUsize,
}

impl CrashLedger {
    pub fn new(max: usize, window: Duration) -> Self {
        CrashLedger {
            max,
            window,
            crashes: Mutex::new(VecDeque::new()),
            disabled: Mutex::new(None),
            last: Mutex::new(None),
            restarts: AtomicUsize::new(0),
        }
    }

    pub fn max(&self) -> usize {
        self.max
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    /// Failures within the window.
    pub fn recent(&self) -> usize {
        let mut c = lock(&self.crashes);
        while c.front().is_some_and(|t| t.elapsed() > self.window) {
            c.pop_front();
        }
        c.len()
    }

    /// Records a failure; when it is the `max`-th within the window the child is quarantined with the
    /// reason `quarantine(failures, reason)` builds. Returns whether it is quarantined now.
    pub fn record_failure(&self, reason: &str, quarantine: impl FnOnce(usize, &str) -> String) -> bool {
        lock(&self.crashes).push_back(Instant::now());
        *lock(&self.last) = Some(reason.to_string());
        let n = self.recent();
        if n >= self.max {
            *lock(&self.disabled) = Some(quarantine(n, reason));
            true
        } else {
            false
        }
    }

    /// Why the child is quarantined, if it is.
    pub fn disabled_reason(&self) -> Option<String> {
        lock(&self.disabled).clone()
    }

    /// The last failure.
    pub fn last_failure(&self) -> Option<String> {
        lock(&self.last).clone()
    }

    /// Lifts the quarantine and forgets the failures.
    pub fn reset(&self) {
        lock(&self.crashes).clear();
        *lock(&self.disabled) = None;
    }

    pub fn since_last_crash(&self) -> Option<Duration> {
        lock(&self.crashes).back().map(Instant::elapsed)
    }

    pub fn count_restart(&self) {
        self.restarts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn restarts(&self) -> usize {
        self.restarts.load(Ordering::Relaxed)
    }

    /// Sleeps out the backoff owed after recent failures (none after a clean history).
    pub fn wait_backoff(&self, base: Duration, max: Duration) {
        let wait = backoff_delay(base, max, self.recent());
        if let Some(since) = self.since_last_crash() {
            if since < wait {
                std::thread::sleep(wait - since);
            }
        }
    }
}

/// Delay before a restart after `recent` failures: `base`, doubling per failure, at most `max`; zero
/// after none.
pub fn backoff_delay(base: Duration, max: Duration, recent: usize) -> Duration {
    if recent == 0 {
        return Duration::ZERO;
    }
    (base * 2u32.saturating_pow(recent.min(31) as u32 - 1)).min(max)
}

/// A live connection to one child.
pub trait Connection: Send + Sync {
    fn pid(&self) -> Option<u32>;
    /// Stops the child immediately and reaps it.
    fn kill(&self);
}

#[derive(Clone)]
pub struct SupervisorConfig {
    /// Names the child in messages ("plugin \"x\"").
    pub label: String,
    pub max_crashes: usize,
    pub crash_window: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

impl SupervisorConfig {
    pub fn new(label: impl Into<String>) -> Self {
        SupervisorConfig {
            label: label.into(),
            max_crashes: 3,
            crash_window: Duration::from_secs(600),
            backoff_base: Duration::from_millis(500),
            backoff_max: Duration::from_secs(8),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChildState {
    /// Not started yet, or stopped after a failure (restarted on next use).
    Stopped,
    Running,
    /// No more restarts until reset.
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildStatus {
    pub state: ChildState,
    pub pid: Option<u32>,
    pub crashes_in_window: usize,
    pub restarts: usize,
    /// Why it is quarantined, or the last failure.
    pub reason: Option<String>,
}

type Launch<C> = Box<dyn Fn() -> Result<C, IpcError> + Send + Sync>;

/// A connection handed to a caller, tagged with the generation it belongs to.
pub struct Lease<C> {
    pub conn: Arc<C>,
    generation: u64,
}

/// Supervises one child; see the module docs. Calls run concurrently on the shared connection; a failure
/// restarts the child once, however many calls saw it.
pub struct Supervisor<C: Connection> {
    cfg: SupervisorConfig,
    ledger: Arc<CrashLedger>,
    launch: Launch<C>,
    current: Mutex<Option<(u64, Arc<C>)>>,
    generation: AtomicU64,
}

impl<C: Connection> Supervisor<C> {
    pub fn new(cfg: SupervisorConfig, launch: impl Fn() -> Result<C, IpcError> + Send + Sync + 'static) -> Self {
        let ledger = Arc::new(CrashLedger::new(cfg.max_crashes, cfg.crash_window));
        Supervisor {
            cfg,
            ledger,
            launch: Box::new(launch),
            current: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    pub fn ledger(&self) -> &Arc<CrashLedger> {
        &self.ledger
    }

    pub fn label(&self) -> &str {
        &self.cfg.label
    }

    fn quarantined(&self) -> Result<(), IpcError> {
        match self.ledger.disabled_reason() {
            Some(r) => Err(IpcError::Spawn(r)),
            None => Ok(()),
        }
    }

    /// The live connection, starting the child if there is none (after the backoff owed).
    pub fn lease(&self) -> Result<Lease<C>, IpcError> {
        self.quarantined()?;
        let mut current = lock(&self.current);
        if let Some((generation, conn)) = current.as_ref() {
            return Ok(Lease {
                conn: conn.clone(),
                generation: *generation,
            });
        }
        self.ledger.wait_backoff(self.cfg.backoff_base, self.cfg.backoff_max);
        self.quarantined()?;
        match (self.launch)() {
            Ok(conn) => {
                let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
                let conn = Arc::new(conn);
                *current = Some((generation, conn.clone()));
                Ok(Lease { conn, generation })
            }
            Err(e) => {
                drop(current);
                self.record(&e);
                Err(e)
            }
        }
    }

    fn record(&self, e: &IpcError) {
        let label = self.cfg.label.clone();
        let minutes = (self.cfg.crash_window.as_secs() / 60).max(1);
        self.ledger.record_failure(&e.to_string(), |n, reason| {
            format!("{label} failed {n} times in {minutes} minutes (last: {reason}) and was stopped; reset it to try again")
        });
    }

    /// Tells the supervisor a call on `lease` failed. A process failure (anything but a typed remote
    /// error) kills that child and counts a failure, once per child however many calls report it.
    pub fn report(&self, lease: &Lease<C>, e: &IpcError) {
        if !e.is_process_failure() {
            return;
        }
        let dead = {
            let mut current = lock(&self.current);
            match current.as_ref() {
                Some((g, _)) if *g == lease.generation => current.take(),
                _ => None,
            }
        };
        if let Some((_, conn)) = dead {
            conn.kill();
            self.ledger.count_restart();
            self.record(e);
        }
    }

    /// Runs `f` on the live connection (starting the child if needed); a process failure is reported.
    pub fn call<R>(&self, f: impl FnOnce(&C) -> Result<R, IpcError>) -> Result<R, IpcError> {
        let lease = self.lease()?;
        let r = f(&lease.conn);
        if let Err(e) = &r {
            self.report(&lease, e);
        }
        r
    }

    /// The child's state now (does not start it).
    pub fn status(&self) -> ChildStatus {
        let current = lock(&self.current);
        let disabled = self.ledger.disabled_reason();
        ChildStatus {
            state: if disabled.is_some() {
                ChildState::Quarantined
            } else if current.is_some() {
                ChildState::Running
            } else {
                ChildState::Stopped
            },
            pid: current.as_ref().and_then(|(_, c)| c.pid()),
            crashes_in_window: self.ledger.recent(),
            restarts: self.ledger.restarts(),
            reason: disabled.or_else(|| self.ledger.last_failure()),
        }
    }

    /// Stops the child (it is started again on next use).
    pub fn shutdown(&self) {
        if let Some((_, conn)) = lock(&self.current).take() {
            conn.kill();
        }
    }

    /// Lifts a quarantine.
    pub fn reset(&self) {
        self.ledger.reset();
    }
}

impl<C: Connection> Drop for Supervisor<C> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct Fake {
        pid: u32,
        killed: Arc<AtomicBool>,
    }

    impl Connection for Fake {
        fn pid(&self) -> Option<u32> {
            Some(self.pid)
        }
        fn kill(&self) {
            self.killed.store(true, Ordering::SeqCst);
        }
    }

    fn cfg() -> SupervisorConfig {
        SupervisorConfig {
            backoff_base: Duration::from_millis(1),
            backoff_max: Duration::from_millis(4),
            ..SupervisorConfig::new("child \"t\"")
        }
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let (b, m) = (Duration::from_millis(500), Duration::from_secs(8));
        assert_eq!(backoff_delay(b, m, 0), Duration::ZERO);
        assert_eq!(backoff_delay(b, m, 1), b);
        assert_eq!(backoff_delay(b, m, 3), Duration::from_secs(2));
        assert_eq!(backoff_delay(b, m, 40), m);
    }

    #[test]
    fn the_ledger_quarantines_after_max_failures_in_the_window() {
        let l = CrashLedger::new(2, Duration::from_secs(60));
        assert_eq!((l.max(), l.window()), (2, Duration::from_secs(60)));
        assert!(!l.record_failure("a", |_, _| unreachable!()));
        assert!(l.record_failure("b", |n, r| format!("{n}:{r}")));
        assert_eq!(l.disabled_reason().as_deref(), Some("2:b"));
        assert_eq!(l.last_failure().as_deref(), Some("b"));
        assert!(l.since_last_crash().is_some());
        l.reset();
        assert_eq!((l.recent(), l.disabled_reason()), (0, None));
        // Old failures leave the window.
        let l = CrashLedger::new(2, Duration::from_millis(5));
        l.record_failure("a", |_, _| String::new());
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(l.recent(), 0);
        l.wait_backoff(Duration::from_millis(1), Duration::from_millis(1));
    }

    #[test]
    fn children_start_lazily_restart_after_failures_and_are_quarantined() {
        let launches = Arc::new(AtomicU64::new(0));
        let killed = Arc::new(AtomicBool::new(false));
        let (l2, k2) = (launches.clone(), killed.clone());
        let sup = Supervisor::new(cfg(), move || {
            let n = l2.fetch_add(1, Ordering::SeqCst) as u32;
            Ok(Fake {
                pid: 100 + n,
                killed: k2.clone(),
            })
        });
        assert_eq!(sup.label(), "child \"t\"");
        assert_eq!(sup.status().state, ChildState::Stopped);
        assert_eq!(launches.load(Ordering::SeqCst), 0, "lazy");
        assert_eq!(sup.call(|c| Ok(c.pid())).unwrap(), Some(100));
        assert_eq!(sup.status().state, ChildState::Running);
        assert_eq!(sup.status().pid, Some(100));
        // A typed remote error is not a failure of the child.
        let remote = IpcError::Remote(crate::RemoteError::new(1, "no"));
        assert_eq!(sup.call(|_| Err::<(), _>(remote.clone())).unwrap_err(), remote);
        assert_eq!(sup.status().crashes_in_window, 0);
        // A crash kills it; the next call starts a new one.
        assert_eq!(
            sup.call(|_| Err::<(), _>(IpcError::Died("x".into()))).unwrap_err(),
            IpcError::Died("x".into())
        );
        assert!(killed.load(Ordering::SeqCst));
        assert_eq!(sup.status().state, ChildState::Stopped);
        assert_eq!(sup.call(|c| Ok(c.pid())).unwrap(), Some(101));
        // Two calls seeing the same dead child count once.
        let lease = sup.lease().unwrap();
        sup.report(&lease, &IpcError::Timeout);
        sup.report(&lease, &IpcError::Timeout);
        assert_eq!(sup.status().crashes_in_window, 2);
        assert_eq!(sup.status().restarts, 2);
        // The third failure quarantines.
        let _ = sup.call(|_| Err::<(), _>(IpcError::Protocol("garbage".into())));
        let st = sup.status();
        assert_eq!(st.state, ChildState::Quarantined);
        assert!(st.reason.as_deref().unwrap().contains("failed 3 times"), "{st:?}");
        let n = launches.load(Ordering::SeqCst);
        assert!(matches!(sup.call(|_| Ok(())), Err(IpcError::Spawn(_))));
        assert_eq!(launches.load(Ordering::SeqCst), n, "no restart while quarantined");
        sup.reset();
        assert!(sup.call(|_| Ok(())).is_ok());
        sup.shutdown();
        assert_eq!(sup.status().state, ChildState::Stopped);
    }

    #[test]
    fn launch_failures_count_and_quarantine() {
        let sup: Supervisor<Fake> = Supervisor::new(cfg(), || Err(IpcError::Spawn("no such file".into())));
        for _ in 0..3 {
            assert!(matches!(sup.lease(), Err(IpcError::Spawn(_))));
        }
        assert_eq!(sup.status().state, ChildState::Quarantined);
        assert!(sup.status().reason.unwrap().contains("no such file"));
    }
}
