//! A running import: progress, cancel, wait.

use super::{ImportReport, StoreError, StoreResult};
use crate::workspace::{ImportProgress, ProgressSnapshot};
use std::sync::Arc;
use std::thread::JoinHandle;

type Snapshot = Box<dyn Fn() -> ProgressSnapshot + Send + Sync>;
type Cancel = Box<dyn Fn() + Send + Sync>;

/// Where a session's progress comes from.
enum Progress {
    /// The job runs in this process.
    Local(Arc<ImportProgress>),
    /// The job runs elsewhere (the store worker): progress and cancel are calls.
    Remote { snapshot: Snapshot, cancel: Cancel },
}

/// An import job running on its own thread. Dropping the session does not stop the job; call
/// [`ImportSession::cancel`] for that.
pub struct ImportSession {
    progress: Progress,
    handle: JoinHandle<StoreResult<ImportReport>>,
}

impl ImportSession {
    /// Runs `work` on a new thread; it reports through `progress`, which the session reads.
    pub fn spawn(
        progress: Arc<ImportProgress>,
        work: impl FnOnce(&ImportProgress) -> StoreResult<ImportReport> + Send + 'static,
    ) -> StoreResult<ImportSession> {
        let p = progress.clone();
        let handle = std::thread::Builder::new()
            .name("debrief-import".into())
            .spawn(move || work(&p))
            .map_err(|e| StoreError::Engine(format!("could not start the import: {e}")))?;
        Ok(ImportSession {
            progress: Progress::Local(progress),
            handle,
        })
    }

    /// A session whose job runs in another process: `wait` waits for it there (on a thread of this one),
    /// `snapshot` and `cancel` ask that process.
    pub fn remote(
        snapshot: impl Fn() -> ProgressSnapshot + Send + Sync + 'static,
        cancel: impl Fn() + Send + Sync + 'static,
        wait: impl FnOnce() -> StoreResult<ImportReport> + Send + 'static,
    ) -> StoreResult<ImportSession> {
        let handle = std::thread::Builder::new()
            .name("debrief-import-wait".into())
            .spawn(wait)
            .map_err(|e| StoreError::Engine(format!("could not follow the import: {e}")))?;
        Ok(ImportSession {
            progress: Progress::Remote {
                snapshot: Box::new(snapshot),
                cancel: Box::new(cancel),
            },
            handle,
        })
    }

    pub fn progress(&self) -> ProgressSnapshot {
        match &self.progress {
            Progress::Local(p) => p.snapshot(),
            Progress::Remote { snapshot, .. } => snapshot(),
        }
    }

    /// Stops the import after the batch in flight; the file being read is not imported.
    pub fn cancel(&self) {
        match &self.progress {
            Progress::Local(p) => p.cancel(),
            Progress::Remote { cancel, .. } => cancel(),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Waits for the end of the import. A crash of the job is an error, not a panic here.
    pub fn wait(self) -> StoreResult<ImportReport> {
        self.handle.join().unwrap_or_else(|payload| {
            Err(StoreError::Engine(format!(
                "the import crashed: {}",
                crate::guard::panic_message(payload.as_ref())
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_waits_and_contains_crashes() {
        let progress = Arc::new(ImportProgress::default());
        let s = ImportSession::spawn(progress.clone(), |p| {
            p.start(1, 10);
            p.file_done(10);
            p.finish();
            Ok(ImportReport {
                events: 3,
                ..ImportReport::default()
            })
        })
        .unwrap();
        let report = s.wait().unwrap();
        assert_eq!(report.events, 3);
        assert_eq!(progress.snapshot().done, 1);

        let s = ImportSession::spawn(Arc::new(ImportProgress::default()), |p| {
            while !p.is_cancelled() {
                std::thread::yield_now();
            }
            Err(StoreError::Cancelled)
        })
        .unwrap();
        assert!(!s.progress().running);
        s.cancel();
        assert_eq!(s.wait().unwrap_err(), StoreError::Cancelled);

        // A job in another process.
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c = cancelled.clone();
        let s = ImportSession::remote(
            ProgressSnapshot::stopped,
            move || c.store(true, std::sync::atomic::Ordering::SeqCst),
            || Ok(ImportReport::default()),
        )
        .unwrap();
        assert!(!s.progress().running);
        s.cancel();
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(s.wait().unwrap().events, 0);

        let s = ImportSession::spawn(Arc::new(ImportProgress::default()), |_| panic!("boom")).unwrap();
        while !s.is_finished() {
            std::thread::yield_now();
        }
        let e = s.wait().unwrap_err();
        assert!(e.to_string().contains("boom"), "{e}");
    }
}
