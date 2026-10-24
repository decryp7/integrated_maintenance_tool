//! One app-wide background run of a tool (the process monitor, the Event Log follower), with its life cycle in one
//! place: `idle → running → ended(outcome) → running …`. A run that ends on its own (an error, a crash) is noticed on
//! the next look and becomes `ended`: the status says why, Stop reports it once, and Start begins a new run instead of
//! saying "already running" about a run that is over.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// A background run as the slot needs it.
pub(crate) trait Run: Send {
    /// Whether it still runs.
    fn is_running(&self) -> bool;
    /// Its status now (kept as the last word once it ended).
    fn snapshot(&self) -> Value;
    /// Stops it (when still running) and waits for it; its failure, if any.
    fn finish(self) -> Result<()>;
}

enum Phase<R> {
    Idle,
    Running { run: R, dir: PathBuf },
    Ended { dir: PathBuf, error: Option<String>, last: Value },
}

pub(crate) struct RunSlot<R: Run> {
    phase: Mutex<Phase<R>>,
}

/// What [`RunSlot::status`] sees.
pub(crate) enum Seen<'a, R> {
    Idle,
    Running {
        run: &'a R,
        dir: &'a Path,
    },
    Ended {
        dir: &'a Path,
        error: Option<&'a str>,
        last: &'a Value,
    },
}

impl<R: Run> Default for RunSlot<R> {
    fn default() -> Self {
        RunSlot {
            phase: Mutex::new(Phase::Idle),
        }
    }
}

impl<R: Run> RunSlot<R> {
    /// The phase, with a run that ended on its own moved to `ended` first.
    fn settled(&self) -> MutexGuard<'_, Phase<R>> {
        let mut g = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(&*g, Phase::Running { run, .. } if !run.is_running()) {
            if let Phase::Running { run, dir } = std::mem::replace(&mut *g, Phase::Idle) {
                let last = run.snapshot();
                let error = run.finish().err().map(|e| format!("{e:#}"));
                *g = Phase::Ended { dir, error, last };
            }
        }
        g
    }

    /// Starts a run with `start` unless one runs (`already` is the refusal). A run that ended is replaced.
    pub fn begin(&self, already: &str, start: impl FnOnce() -> Result<(R, PathBuf)>) -> Result<()> {
        let mut g = self.settled();
        if matches!(&*g, Phase::Running { .. }) {
            bail!("{already}");
        }
        let (run, dir) = start()?;
        *g = Phase::Running { run, dir };
        Ok(())
    }

    /// Stops the run: its folder, `None` when nothing ran, or the failure of a run that ended on its own (once).
    pub fn end(&self) -> Result<Option<String>> {
        let phase = std::mem::replace(&mut *self.settled(), Phase::Idle);
        match phase {
            Phase::Idle => Ok(None),
            Phase::Running { run, dir } => {
                run.finish()?;
                Ok(Some(dir.to_string_lossy().into_owned()))
            }
            Phase::Ended { error: Some(e), .. } => Err(anyhow!("{e}")),
            Phase::Ended { dir, error: None, .. } => Ok(Some(dir.to_string_lossy().into_owned())),
        }
    }

    /// Looks at the phase (a run that ended on its own is seen as ended).
    pub fn look<T>(&self, f: impl FnOnce(Seen<'_, R>) -> T) -> T {
        let g = self.settled();
        match &*g {
            Phase::Idle => f(Seen::Idle),
            Phase::Running { run, dir } => f(Seen::Running { run, dir }),
            Phase::Ended { dir, error, last } => f(Seen::Ended {
                dir,
                error: error.as_deref(),
                last,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct Fake {
        running: Arc<AtomicBool>,
        fails: bool,
    }

    impl Run for Fake {
        fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }
        fn snapshot(&self) -> Value {
            json!({ "events": 3 })
        }
        fn finish(self) -> Result<()> {
            if self.fails {
                bail!("write failed")
            }
            Ok(())
        }
    }

    fn start(slot: &RunSlot<Fake>, fails: bool) -> Result<Arc<AtomicBool>> {
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        slot.begin("already running", || Ok((Fake { running: r, fails }, PathBuf::from("/d"))))?;
        Ok(running)
    }

    #[test]
    fn idle_running_stopped() {
        let slot = RunSlot::default();
        assert!(slot.look(|s| matches!(s, Seen::Idle)));
        start(&slot, false).unwrap();
        assert_eq!(start(&slot, false).unwrap_err().to_string(), "already running");
        assert!(slot.look(|s| matches!(s, Seen::Running { .. })));
        assert_eq!(slot.end().unwrap().as_deref(), Some("/d"));
        assert_eq!(slot.end().unwrap(), None);
    }

    #[test]
    fn a_run_that_ended_on_its_own_is_ended_reports_once_and_does_not_block_a_new_start() {
        let slot = RunSlot::default();
        start(&slot, true).unwrap().store(false, Ordering::SeqCst);
        slot.look(|s| match s {
            Seen::Ended { error, last, .. } => {
                assert_eq!(error, Some("write failed"));
                assert_eq!(last["events"], 3, "the last status is kept");
            }
            _ => panic!("not ended"),
        });
        assert_eq!(slot.end().unwrap_err().to_string(), "write failed");
        assert_eq!(slot.end().unwrap(), None, "reported once");
        start(&slot, true).unwrap().store(false, Ordering::SeqCst);
        start(&slot, false).expect("an ended run is replaced");
        assert!(slot.look(|s| matches!(s, Seen::Running { .. })));
    }
}
