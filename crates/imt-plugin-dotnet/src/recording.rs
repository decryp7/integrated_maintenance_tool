//! A recording running on its own thread, shared by the ETW and EventPipe
//! backends: the backend's loop watches a [`Control`] (stop, cancel, limits)
//! and reports its size; [`ThreadRecording`] is the [`crate::Recording`] the
//! caller holds.

use crate::{DotnetError, Finished, Recording};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Shared between the caller and the recording thread.
#[derive(Debug, Default)]
pub struct Control {
    stop: AtomicBool,
    cancel: AtomicBool,
    bytes: AtomicU64,
    done: AtomicBool,
}

impl Control {
    pub fn new() -> Arc<Control> {
        Arc::new(Control::default())
    }

    /// Whether the recording should end (stop or cancel).
    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.request_stop();
    }

    pub fn set_bytes(&self, n: u64) {
        self.bytes.store(n, Ordering::Release);
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Acquire)
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    fn set_done(&self) {
        self.done.store(true, Ordering::Release);
    }
}

/// Sleeps in short steps until `deadline` or until `ctl` is stopping; returns whether it was stopped.
pub fn wait_or_stop(ctl: &Control, deadline: Instant) -> bool {
    while Instant::now() < deadline {
        if ctl.stopping() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())));
    }
    ctl.stopping()
}

/// A recording thread and its control.
pub struct ThreadRecording {
    ctl: Arc<Control>,
    started: Instant,
    thread: Option<JoinHandle<Result<Finished, DotnetError>>>,
    out: PathBuf,
}

impl ThreadRecording {
    /// Runs `body` on a new thread; it must return when `ctl` is stopping (or by itself, e.g. at a limit).
    pub fn spawn(
        name: &str,
        out: PathBuf,
        body: impl FnOnce(Arc<Control>) -> Result<Finished, DotnetError> + Send + 'static,
    ) -> Result<ThreadRecording, DotnetError> {
        let ctl = Control::new();
        let c = ctl.clone();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(c.clone())))
                    .unwrap_or_else(|_| Err(DotnetError::Protocol("the recording failed unexpectedly (a bug in IMT)".into())));
                c.set_done();
                r
            })
            .map_err(DotnetError::Io)?;
        Ok(ThreadRecording {
            ctl,
            started: Instant::now(),
            thread: Some(thread),
            out,
        })
    }

    fn join(&mut self) -> Result<Finished, DotnetError> {
        match self.thread.take() {
            Some(t) => t
                .join()
                .unwrap_or_else(|_| Err(DotnetError::Protocol("the recording thread panicked".into()))),
            None => Err(DotnetError::Protocol("the recording was already stopped".into())),
        }
    }
}

impl Recording for ThreadRecording {
    fn bytes(&self) -> u64 {
        self.ctl.bytes()
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn is_finished(&self) -> bool {
        self.ctl.is_done()
    }

    fn stop(mut self: Box<Self>) -> Result<Finished, DotnetError> {
        self.ctl.request_stop();
        self.join()
    }

    fn cancel(mut self: Box<Self>) {
        self.ctl.request_cancel();
        let _ = self.join();
        let _ = std::fs::remove_file(&self.out);
    }
}

impl Drop for ThreadRecording {
    fn drop(&mut self) {
        // A recording is never left running (an ETW session would outlive IMT).
        if self.thread.is_some() {
            self.ctl.request_stop();
            let _ = self.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(path: PathBuf, bytes: u64) -> Finished {
        Finished {
            path,
            bytes,
            truncated: false,
            notes: Vec::new(),
        }
    }

    #[test]
    fn stop_joins_and_returns_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.nettrace");
        let o = out.clone();
        let r = ThreadRecording::spawn("t", out.clone(), move |ctl| {
            ctl.set_bytes(7);
            while !ctl.stopping() {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::fs::write(&o, b"x").unwrap();
            Ok(finished(o, 7))
        })
        .unwrap();
        let start = Instant::now();
        while r.bytes() != 7 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(r.bytes(), 7);
        assert!(!r.is_finished());
        assert!(r.elapsed() < Duration::from_secs(60));
        let f = Box::new(r).stop().unwrap();
        assert_eq!(f.bytes, 7);
        assert!(out.exists());
    }

    #[test]
    fn cancel_deletes_the_file_and_panics_become_errors() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("b.nettrace");
        std::fs::write(&out, b"partial").unwrap();
        let r = ThreadRecording::spawn("t", out.clone(), |ctl| {
            assert!(wait_or_stop(&ctl, Instant::now() + Duration::from_secs(30)));
            assert!(ctl.cancelled());
            Err(DotnetError::Cancelled)
        })
        .unwrap();
        Box::new(r).cancel();
        assert!(!out.exists());

        let r = ThreadRecording::spawn("t", out.clone(), |_| panic!("boom")).unwrap();
        let start = Instant::now();
        while !r.is_finished() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(r.is_finished());
        assert!(matches!(Box::new(r).stop(), Err(DotnetError::Protocol(_))));

        // Dropping a running recording stops it.
        let r = ThreadRecording::spawn("t", out, |ctl| {
            wait_or_stop(&ctl, Instant::now() + Duration::from_secs(30));
            Err(DotnetError::Cancelled)
        })
        .unwrap();
        drop(r);
        assert!(!wait_or_stop(&Control::default(), Instant::now()));
    }
}
