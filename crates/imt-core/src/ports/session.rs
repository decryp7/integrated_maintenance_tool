//! A running import: progress, cancel, wait.

use super::{ImportReport, StoreError, StoreResult};
use crate::workspace::{ImportProgress, ProgressSnapshot};
use std::sync::Arc;
use std::thread::JoinHandle;

/// An import job running on its own thread. Dropping the session does not stop the job; call
/// [`ImportSession::cancel`] for that.
pub struct ImportSession {
    progress: Arc<ImportProgress>,
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
            .name("imt-import".into())
            .spawn(move || work(&p))
            .map_err(|e| StoreError::Engine(format!("could not start the import: {e}")))?;
        Ok(ImportSession { progress, handle })
    }

    pub fn progress(&self) -> ProgressSnapshot {
        self.progress.snapshot()
    }

    /// Stops the import after the batch in flight; the file being read is not imported.
    pub fn cancel(&self) {
        self.progress.cancel();
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

        let s = ImportSession::spawn(Arc::new(ImportProgress::default()), |_| panic!("boom")).unwrap();
        while !s.is_finished() {
            std::thread::yield_now();
        }
        let e = s.wait().unwrap_err();
        assert!(e.to_string().contains("boom"), "{e}");
    }
}
