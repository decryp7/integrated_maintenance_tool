//! Imports as jobs (ADR 0012, 0014): opening files, reloading, refreshing and re-deriving after a profile change
//! run in the store (the worker) as an [`ImportSession`]; the window's import job follows it: its progress (files,
//! bytes, the current file) is the job's, Cancel cancels the session, and its report is the result. Readers keep
//! working meanwhile (the store never blocks them). The annotators run over the new data before the job ends.

use crate::jobs::{Controls, Done, JobCtx, JobPhase, JobProgress, JobResult, JobSpec, PhaseScale};
use crate::windows::{CurrentWindow, WindowState};
use crate::CmdResult;
use debrief_core::ports::{ImportReport, ImportSession, Store, StoreResult};
use debrief_core::workspace::ProgressSnapshot;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Runtime};

/// How often a running import's progress is read.
const POLL: Duration = Duration::from_millis(100);

/// What starts the import in the store (called again while another import, e.g. live tail's, still runs).
pub(crate) type Start = Box<dyn Fn(&dyn Store) -> StoreResult<ImportSession> + Send>;

/// How long an import waits for the store's running import (live tail, another window's job) to end.
const BUSY_WAIT: Duration = Duration::from_secs(300);

/// Starts the import, waiting while the store is busy with another one (the job says so); Cancel stops waiting.
fn start_when_free(store: &dyn Store, start: &Start, ctx: &JobCtx, cancel: &AtomicBool) -> StoreResult<ImportSession> {
    let began = std::time::Instant::now();
    let mut told = false;
    loop {
        match start(store) {
            Err(debrief_core::ports::StoreError::Busy(_)) if began.elapsed() < BUSY_WAIT => {
                if ctx.is_cancelled() || cancel.load(Ordering::Relaxed) {
                    return Err(debrief_core::ports::StoreError::Cancelled);
                }
                if !told {
                    ctx.message("Waiting for the running import");
                    told = true;
                }
                std::thread::sleep(POLL);
            }
            other => return other,
        }
    }
}

/// The job's progress from the session's.
pub(crate) fn job_progress(p: &ProgressSnapshot) -> JobProgress {
    JobProgress {
        phase: p.phase.as_deref().map(|l| JobPhase::new(l, PhaseScale::Indeterminate)),
        fraction: (p.bytes_total > 0 && p.phase.is_none()).then(|| (p.bytes_done as f64 / p.bytes_total as f64) as f32),
        done: Some(p.done as u64),
        total: Some(p.total as u64),
        eta_ms: p.eta_ms,
        bytes_done: Some(p.bytes_done),
        bytes_total: (p.bytes_total > 0).then_some(p.bytes_total),
        current: p.current.as_deref().map(file_name),
        ..JobProgress::default()
    }
}

/// A file's name for the job's progress line (the path stays out of the jobs list).
fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// Follows `session` until it ends: progress to the job and the window, the job's Cancel to the session.
pub(crate) fn follow(win: &WindowState, ctx: &JobCtx, cancel: &AtomicBool, session: ImportSession) -> StoreResult<ImportReport> {
    let mut cancelled = false;
    while !session.is_finished() {
        if !cancelled && (ctx.is_cancelled() || cancel.load(Ordering::Relaxed)) {
            log::info!("Import cancelled by the user");
            session.cancel();
            cancelled = true;
        }
        let p = session.progress();
        ctx.progress(job_progress(&p));
        win.engine.note_progress(p);
        std::thread::sleep(POLL);
    }
    win.engine.note_progress(ProgressSnapshot::stopped());
    let report = session.wait()?;
    win.engine.note_report(report.clone());
    Ok(report)
}

/// Runs an import as the window's import job and waits for its report (one import at a time per window; the
/// Jobs pane shows its progress, Cancel stops it).
pub(crate) async fn run<R: Runtime>(app: &AppHandle<R>, state: &CurrentWindow, spec: JobSpec, start: Start) -> CmdResult<ImportReport> {
    let (controls, flag) = controls();
    let win = state.arc().clone();
    crate::jobs::run(
        app,
        spec.window(state.label()),
        controls,
        move |ctx: &JobCtx| -> CmdResult<Done<ImportReport>> {
            let store = win.engine.store()?;
            let session = start_when_free(store.as_ref(), &start, ctx, &flag)?;
            let report = follow(&win, ctx, &flag, session)?;
            super::annotate::run(&win, store.as_ref(), &|m| ctx.message(m));
            Ok(Done::new(report).with_result(JobResult::Workspace))
        },
    )
    .await
}

/// Starts an import job without waiting for it (a profile change re-derives in the background); `then` runs
/// with the report once it is done, and its failure fails the job. Returns the job's id.
pub(crate) fn start<R: Runtime>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    spec: JobSpec,
    start: Start,
    then: impl FnOnce(ImportReport) -> CmdResult<()> + Send + 'static,
) -> CmdResult<u64> {
    let (controls, flag) = controls();
    let win = state.arc().clone();
    crate::error::lift(
        crate::jobs::spawn(
            app,
            spec.window(state.label()),
            controls,
            move |ctx: &JobCtx| -> CmdResult<Done<()>> {
                let store = win.engine.store()?;
                let session = start_when_free(store.as_ref(), &start, ctx, &flag)?;
                let report = follow(&win, ctx, &flag, session)?;
                super::annotate::run(&win, store.as_ref(), &|m| ctx.message(m));
                then(report)?;
                Ok(Done::new(()).with_result(JobResult::Workspace))
            },
        )
        .map(|id| id.0),
    )
}

/// The job's Cancel sets a flag the follower passes to the session.
fn controls() -> (Controls, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    (Controls::cancel(move || f.store(true, Ordering::Relaxed)), flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_job_shows_files_bytes_and_the_current_file() {
        let p = ProgressSnapshot {
            running: true,
            done: 2,
            total: 5,
            cancelled: false,
            bytes_done: 50,
            bytes_total: 200,
            current: Some("/logs/app.log".into()),
            elapsed_ms: 10,
            eta_ms: Some(30),
            phase: None,
        };
        let j = job_progress(&p);
        assert_eq!(j.fraction, Some(0.25));
        assert_eq!(
            (j.done, j.total, j.bytes_done, j.bytes_total),
            (Some(2), Some(5), Some(50), Some(200))
        );
        assert_eq!(j.current.as_deref(), Some("app.log"));
        let idle = job_progress(&ProgressSnapshot::stopped());
        assert_eq!((idle.fraction, idle.bytes_total, idle.current), (None, None, None));
        let tail = job_progress(&ProgressSnapshot {
            phase: Some("Compacting segments".into()),
            ..p.clone()
        });
        assert_eq!(tail.fraction, None, "the tail has no byte scale");
        assert_eq!(tail.phase, Some(JobPhase::new("Compacting segments", PhaseScale::Indeterminate)));
    }
}
