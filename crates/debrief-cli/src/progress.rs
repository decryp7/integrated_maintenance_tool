//! Phase progress of an import on standard error, and Ctrl+C: the import is cancelled (the files read so far stay in
//! the store) and the process exits with 130.

use crate::watch::stop_flag;
use debrief_core::ports::{ImportReport, ImportSession, StoreResult};
use debrief_core::workspace::{ImportPhase, ProgressSnapshot};
use std::io::{IsTerminal, Write};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// How often the import is looked at (a Ctrl+C is noticed within this).
const POLL: Duration = Duration::from_millis(100);
/// Progress lines are written at most this often.
const LINE_EVERY: Duration = Duration::from_millis(500);

/// The message of an import that Ctrl+C stopped (mapped to [`crate::Exit::Interrupted`]).
pub const INTERRUPTED: &str = "interrupted: the import was cancelled, the files read so far are kept";

/// One short line for a snapshot: the phase and its counts ("Importing 12 of 40 files, 3.2 GB of 9.1 GB").
pub fn progress_line(p: &ProgressSnapshot) -> String {
    let size = |b: u64| {
        let (mut v, mut u) = (b as f64, 0);
        while v >= 1024.0 && u < 4 {
            v /= 1024.0;
            u += 1;
        }
        format!("{v:.1} {}", ["B", "KB", "MB", "GB", "TB"][u])
    };
    match p.phase {
        ImportPhase::Idle => String::new(),
        ImportPhase::Finding => format!("Finding files: {} found ({})", p.done, size(p.bytes_total)),
        ImportPhase::Detecting => format!("Checking files: {} of {}", p.done, p.total),
        ImportPhase::Importing => format!("Importing {} of {} files, {} of {}", p.done, p.total, size(p.bytes_done), size(p.bytes_total)),
        ImportPhase::Finishing => p.stage.clone().unwrap_or_else(|| "Finishing".into()),
    }
}

/// Waits for `session`, writing a progress line to `err` every half second when `show` is set, and cancelling the
/// import when Ctrl+C was pressed. `Err(INTERRUPTED)` then; otherwise the import's own result.
pub fn wait_with_progress(session: ImportSession, show: bool, err: &mut dyn Write) -> Result<StoreResult<ImportReport>, &'static str> {
    wait_until(session, show, err, || stop_flag().load(Ordering::Relaxed))
}

/// [`wait_with_progress`] with the stop request passed in (tests).
pub fn wait_until(
    session: ImportSession,
    show: bool,
    err: &mut dyn Write,
    stop: impl Fn() -> bool,
) -> Result<StoreResult<ImportReport>, &'static str> {
    let (mut last_line, mut last_at, mut interrupted) = (String::new(), None::<Instant>, false);
    while !session.is_finished() {
        if stop() && !interrupted {
            interrupted = true;
            session.cancel();
        }
        if show && !interrupted && last_at.is_none_or(|t| t.elapsed() >= LINE_EVERY) {
            let line = progress_line(&session.progress());
            if !line.is_empty() && line != last_line {
                let _ = writeln!(err, "{line}");
                last_line = line;
            }
            last_at = Some(Instant::now());
        }
        std::thread::sleep(POLL);
    }
    let result = session.wait();
    if interrupted {
        Err(INTERRUPTED)
    } else {
        Ok(result)
    }
}

/// Whether progress lines are wanted: not quiet, and standard error is a terminal (logs and pipes stay clean).
pub fn show_progress(quiet: bool) -> bool {
    !quiet && std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_core::workspace::ImportProgress;
    use std::sync::Arc;

    fn session(work: impl FnOnce(&ImportProgress) -> StoreResult<ImportReport> + Send + 'static) -> ImportSession {
        ImportSession::spawn(Arc::new(ImportProgress::default()), work).unwrap()
    }

    #[test]
    fn lines_name_the_phase_with_its_counts() {
        let p = ImportProgress::default();
        let run = p.begin();
        assert!(progress_line(&p.snapshot()).starts_with("Finding files: 0 found"));
        p.set_stage(Some("Compacting segments"));
        assert_eq!(progress_line(&p.snapshot()), "Compacting segments");
        drop(run);
        assert_eq!(progress_line(&p.snapshot()), "");
    }

    #[test]
    fn a_finished_import_returns_its_report_and_a_stop_cancels_it() {
        let mut out = Vec::new();
        let done = session(|_| Ok(ImportReport { events: 7, ..ImportReport::default() }));
        let report = wait_until(done, false, &mut out, || false).unwrap().unwrap();
        assert_eq!(report.events, 7);
        let slow = session(|p| {
            while !p.is_cancelled_flag() {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(ImportReport::default())
        });
        assert_eq!(wait_until(slow, true, &mut out, || true).unwrap_err(), INTERRUPTED);
        assert!(out.is_empty(), "no progress line once it is being cancelled");
    }

    #[test]
    fn quiet_and_pipes_show_nothing() {
        assert!(!show_progress(true));
        // In the test run standard error is not a terminal.
        assert!(!show_progress(false) || std::io::stderr().is_terminal());
    }
}
