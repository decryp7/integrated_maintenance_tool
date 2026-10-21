//! Jobs: every long-running task of IMT (AI requests, imports, long queries and analyses, exports and
//! sharing, downloads, the folder survey, storage clean-up, diagnostics collections) in one service with
//! per-kind queues and limits, progress, Cancel, Retry and a history saved with the project.
//!
//! - [`model`]: the job types and the state machine (`queued → running → done | failed | cancelled |
//!   interrupted`).
//! - [`queue`]: the pure queue (limits, FIFO, AI reorder, pause, visibility and attention rules).
//! - [`persist`]: the versioned history record (UI state key `jobs`).
//! - [`service`]: threads, stop actions, probes, listeners; Tauri-free, with an injected clock.
//! - Here: the Tauri glue: [`init`], [`run`] for commands, events, the tray mirror, notification-centre
//!   messages for finished jobs, and the commands.
//!
//! This evolves [`crate::background::BackgroundTasks`]: work that ends is a job; what keeps running until
//! stopped (live tail, the process monitor, PerfMon) stays a background task. Running jobs are mirrored
//! into the background tasks, so the tray and the status bar list (and stop) them too.
//!
//! Event: `imt://jobs` (the window's [`JobsSnapshot`]). Wire contract: `app/src/app/jobs/jobs.types.ts`.

pub mod model;
pub mod persist;
pub mod queue;
pub mod service;

pub use model::{JobId, JobKind, JobLink, JobProgress, JobResult, JobSpec, JobState};
pub use queue::JobsSnapshot;
#[cfg(test)]
pub(crate) use service::tests::{wait_until, ManualClock};
pub use service::{Clock, Controls, Done, JobCtx, JobFailure, Jobs, JobsEvent, Stopped, SystemClock};

use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource};
use crate::notify::NotificationKind;
use crate::windows::CurrentWindow;
use crate::CmdResult;
use queue::Finished;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime};

pub const JOBS_EVENT: &str = "imt://jobs";
const TASK_PREFIX: &str = "job:";

/// Creates the service and wires it to the windows, the tray and the notification centre.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let jobs = Jobs::new(Arc::new(SystemClock));
    wire(app, &jobs);
    app.manage(jobs);
}

/// Subscribes the app's listeners (separate from [`init`] so tests can pass a service with a fake clock).
pub fn wire<R: Runtime>(app: &AppHandle<R>, jobs: &Jobs) {
    let handle = app.clone();
    let service = jobs.clone();
    let h = app.clone();
    jobs.set_retry(JobKind::Import, move |job| crate::workspace_commands::retry_import(&h, job));
    let h = app.clone();
    jobs.set_retry(JobKind::Research, move |job| crate::browser::research::retry(&h, job));
    let mirrored = Mutex::new(HashSet::<String>::new());
    let sent = Mutex::new(Sent::default());
    jobs.subscribe(move |event| match event {
        JobsEvent::Changed => {
            emit_all(&handle, &service, &sent);
            mirror(&handle, &service, &mirrored);
        }
        JobsEvent::Finished(f) => finished(&handle, f),
        JobsEvent::Paused { kind, reason } => paused(&handle, *kind, reason),
    });
}

/// What each window was last sent: its version and snapshot.
type Sent = std::collections::HashMap<String, (u64, JobsSnapshot)>;

/// What `imt://jobs` carries: the window's jobs as a change of the last event (version `version`, the one
/// before is `version - 1`): the jobs in their order by id, those new or changed in full, the paused queues.
/// A window that missed a version asks for the list again (`jobs_list`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsDelta {
    pub version: u64,
    pub order: Vec<JobId>,
    pub changed: Vec<queue::JobInfo>,
    pub paused: Vec<queue::PausedKind>,
}

/// The change from `prev` to `next` as version `version`; `None` when nothing changed (nothing is sent).
pub fn delta(prev: Option<&JobsSnapshot>, next: &JobsSnapshot, version: u64) -> Option<JobsDelta> {
    if prev == Some(next) {
        return None;
    }
    let changed = next
        .jobs
        .iter()
        .filter(|j| !prev.is_some_and(|p| p.jobs.iter().any(|o| o == *j)))
        .cloned()
        .collect();
    Some(JobsDelta {
        version,
        order: next.jobs.iter().map(|j| j.id).collect(),
        changed,
        paused: next.paused.clone(),
    })
}

/// Sends each window what changed in its jobs since its last event (nothing when nothing did). `sent` is
/// held only to compute the deltas; events go out after it is released.
fn emit_all<R: Runtime>(app: &AppHandle<R>, jobs: &Jobs, sent: &Mutex<Sent>) {
    let Some(windows) = app.try_state::<crate::windows::Windows>() else {
        return;
    };
    let labels: Vec<String> = windows.all().iter().map(|w| w.label().to_string()).collect();
    let deltas: Vec<(String, JobsDelta)> = {
        let mut sent = crate::sync::lock(sent);
        sent.retain(|l, _| labels.contains(l));
        labels
            .into_iter()
            .filter_map(|label| {
                let next = jobs.snapshot(Some(&label));
                let (version, prev) = sent.get(&label).map_or((0, None), |(v, s)| (*v, Some(s)));
                let d = delta(prev, &next, version + 1)?;
                sent.insert(label.clone(), (version + 1, next));
                Some((label, d))
            })
            .collect()
    };
    for (label, d) in deltas {
        crate::windows::emit_to_window(app, &label, JOBS_EVENT, d);
    }
}

/// Lists running jobs (once visible) as background tasks: the tray and the status bar show and stop them.
/// `mirrored` is only held to swap the listed set: registering tasks notifies their listeners (the tray,
/// the windows), which must never run under it.
pub(crate) fn mirror<R: Runtime>(app: &AppHandle<R>, jobs: &Jobs, mirrored: &Mutex<HashSet<String>>) {
    let Some(tasks) = app.try_state::<BackgroundTasks>() else { return };
    let running = jobs.running_visible();
    let now: HashSet<String> = running.iter().map(|j| format!("{TASK_PREFIX}{}", j.id)).collect();
    let known = std::mem::replace(&mut *crate::sync::lock(mirrored), now.clone());
    for gone in known.difference(&now) {
        tasks.remove(gone);
    }
    for j in running.iter().filter(|j| !known.contains(&format!("{TASK_PREFIX}{}", j.id))) {
        let (service, id) = (jobs.clone(), j.id);
        let stop: Option<crate::background::StopFn> = j.cancellable.then(|| {
            let stop: crate::background::StopFn = Arc::new(move || service.cancel(id).map_err(anyhow::Error::msg));
            stop
        });
        let spec = TaskSpec {
            id: format!("{TASK_PREFIX}{id}"),
            label: j.title.clone(),
            kind: TaskKind::Job,
            window: j.source.window.clone(),
        };
        if let Err(e) = tasks.register(spec, stop) {
            log::debug!("job not listed as background work: {e:#}");
        }
    }
}

/// "Explanation ready" and the like for an AI answer that ran in the background.
fn answer_title(job: &model::Job) -> String {
    match job.request.as_ref().and_then(|r| r["op"].as_str()) {
        Some("explain") => "Explanation ready".into(),
        Some("summarize") => "Summary ready".into(),
        Some("rootCauses") => "Root causes ready".into(),
        _ => format!("Finished: {}", job.title),
    }
}

fn job_title(kind: JobKind, state: JobState, title: &str) -> String {
    match (kind, state) {
        (JobKind::Research, JobState::Done) => format!("Research finished: {}", title.strip_prefix("Research: ").unwrap_or(title)),
        (_, JobState::Done) => format!("Finished: {title}"),
        (_, JobState::Failed) => format!("Failed: {title}"),
        (_, _) => format!("Interrupted: {title}"),
    }
}

/// A finished job that needs attention becomes a message (and an OS notification while IMT is not focused).
fn finished<R: Runtime>(app: &AppHandle<R>, f: &Finished) {
    if !f.attention {
        return;
    }
    let job = &f.job;
    let id = job.id.0;
    let ok = job.state() == JobState::Done;
    let retryable = job.request.is_some();
    let primary = match (&job.result, ok) {
        (Some(JobResult::Proposal { .. }), true) => NoticeAction::ReviewProposals { job_id: id },
        (Some(JobResult::File { .. }), true) => NoticeAction::ShowInFolder { job_id: id },
        (_, true) => NoticeAction::ViewJob { job_id: id },
        (_, false) if retryable => NoticeAction::RetryJob { job_id: id },
        _ => NoticeAction::ShowJobs,
    };
    let body = if ok {
        // What the job reported last ("Bundle written, 5 values redacted"), else how long it took.
        job.message
            .clone()
            .unwrap_or_else(|| format!("Took {} s.", (job.run_ms(0) + 500) / 1000))
    } else {
        job.error.clone().unwrap_or_default()
    };
    let scope = if job.window().is_some() {
        NoticeScope::Project
    } else {
        NoticeScope::App
    };
    let title = match (&job.result, ok) {
        (Some(JobResult::Narrative { .. }), true) => answer_title(job),
        _ => job_title(job.kind, job.state(), &job.title),
    };
    let mut notice = NewNotice::new(
        NoticeSource::Job,
        if ok { NoticeSeverity::Success } else { NoticeSeverity::Error },
        title,
        scope,
    )
    .body(if matches!(job.result, Some(JobResult::Narrative { .. })) && ok {
        job.title.clone()
    } else {
        body
    })
    .primary(primary)
    .dedupe(format!("job:{id}"))
    .link(NoticeLink {
        window: job.source.window.clone(),
        job_id: Some(id),
        ..NoticeLink::default()
    });
    if !ok {
        notice = notice.secondary(NoticeAction::ShowJobs);
    }
    // Imports in a background window already raise their own OS notification (`import_finished`).
    let os = match (job.kind, ok) {
        (JobKind::Import, true) => None,
        (_, true) => Some(NotificationKind::JobFinished),
        (_, false) => Some(NotificationKind::JobFailed),
    };
    crate::notification_centre::post(app, job.window(), notice, os);
}

/// A paused queue (the AI was quarantined): an app message with "Reset AI".
fn paused<R: Runtime>(app: &AppHandle<R>, kind: JobKind, reason: &str) {
    if kind != JobKind::Ai {
        return;
    }
    let notice = NewNotice::new(
        NoticeSource::Ai,
        NoticeSeverity::Warning,
        "AI requests are paused",
        NoticeScope::App,
    )
    .body(format!("{reason}. Waiting requests stay queued until you reset the AI."))
    .primary(NoticeAction::ResetAi)
    .secondary(NoticeAction::ShowJobs)
    .dedupe("ai:paused");
    crate::notification_centre::post(app, None, notice, Some(NotificationKind::Attention));
}

/// Runs `f` as a job and hands its value back to the awaiting command. Without the service (some tests)
/// `f` runs directly. A refused, cancelled-while-queued or interrupted job becomes an `E`.
pub(crate) async fn run<R, T, E>(
    app: &AppHandle<R>,
    spec: JobSpec,
    controls: Controls,
    f: impl FnOnce(&JobCtx) -> Result<Done<T>, E> + Send + 'static,
) -> Result<T, E>
where
    R: Runtime,
    T: Send + 'static,
    E: JobFailure + From<Stopped> + Send + 'static,
{
    run_announced(app, spec, controls, |_| {}, f).await
}

/// Like [`run`], telling `announce` the job's id once it is queued (the caller hands it to the UI, which
/// cancels with `jobs_cancel`: Jobs are the one cancel authority).
pub(crate) async fn run_announced<R, T, E>(
    app: &AppHandle<R>,
    spec: JobSpec,
    controls: Controls,
    announce: impl FnOnce(JobId),
    f: impl FnOnce(&JobCtx) -> Result<Done<T>, E> + Send + 'static,
) -> Result<T, E>
where
    R: Runtime,
    T: Send + 'static,
    E: JobFailure + From<Stopped> + Send + 'static,
{
    let Some(jobs) = app.try_state::<Jobs>().map(|j| j.inner().clone()) else {
        return tauri::async_runtime::spawn_blocking(move || f(&JobCtx::detached()).map(|d| d.value))
            .await
            .map_err(|e| E::from(Stopped::Failed(format!("the task stopped unexpectedly ({e})"))))?;
    };
    let handle = jobs
        .submit(spec, controls, f)
        .map_err(|r| E::from(Stopped::Refused(r.to_string())))?;
    announce(handle.id);
    tauri::async_runtime::spawn_blocking(move || handle.wait())
        .await
        .map_err(|e| E::from(Stopped::Failed(format!("the task stopped unexpectedly ({e})"))))?
        .map_err(E::from)?
}

/// Reads the window's store on the calling (job) thread.
pub(crate) fn read_store<T>(
    win: &crate::windows::WindowState,
    f: impl FnOnce(&dyn imt_core::LogStore) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let ws = win.workspace.clone();
    let guard = crate::sync::read(&ws);
    f(guard.as_ref())
}

/// A read-only analysis of the window's logs as an analysis job (limited to the cores, minus one). Its
/// View opens `view` with the query applied.
pub(crate) async fn analysis<R: Runtime, T: Send + 'static>(
    app: &AppHandle<R>,
    win: &CurrentWindow,
    title: &str,
    view: &str,
    ql: Option<String>,
    controls: Controls,
    f: impl FnOnce(&dyn imt_core::LogStore, &JobCtx) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    analysis_announced(app, win, title, view, ql, controls, |_| {}, f).await
}

/// [`analysis`], telling `announce` the job's id once it is queued (see [`run_announced`]).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn analysis_announced<R: Runtime, T: Send + 'static>(
    app: &AppHandle<R>,
    win: &CurrentWindow,
    title: &str,
    view: &str,
    ql: Option<String>,
    controls: Controls,
    announce: impl FnOnce(JobId),
    f: impl FnOnce(&dyn imt_core::LogStore, &JobCtx) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let mut spec = JobSpec::new(JobKind::Analysis, title).window(win.label());
    let ql = ql.filter(|q| !q.trim().is_empty());
    if let Some(q) = &ql {
        spec = spec.link(JobLink::Query { ql: q.clone() });
    }
    let result = JobResult::View {
        view: view.to_string(),
        ql,
    };
    let w = win.arc().clone();
    crate::error::lift(
        run_announced(app, spec, controls, announce, move |ctx| {
            read_store(&w, |ws| f(ws, ctx))
                .map(|v| Done::new(v).with_result(result))
                .map_err(crate::err)
        })
        .await,
    )
}

/// The spec of an export to `path` by the window (or the app).
pub(crate) fn export_spec(title: &str, window: Option<&str>, path: &std::path::Path) -> JobSpec {
    let name = path
        .file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let mut spec = JobSpec::new(JobKind::Export, format!("{title} {name}")).link(JobLink::Path {
        path: path.display().to_string(),
    });
    if let Some(w) = window {
        spec = spec.window(w);
    }
    spec
}

/// Queues work that writes the file `path` (an export, a bundle, a report, diagnostics) and returns its job at
/// once: the user goes on working and a message with "Show in folder" says when the file is there. `f` returns
/// the line that message shows ("Exported 1,204 events"). Check what can be checked at once (the path, overwrite)
/// before calling this, so those errors reach the dialog that asked.
pub(crate) fn start_file<R: Runtime>(
    app: &AppHandle<R>,
    spec: JobSpec,
    path: &std::path::Path,
    f: impl FnOnce(&JobCtx) -> Result<String, String> + Send + 'static,
) -> CmdResult<u64> {
    let result = JobResult::File {
        path: path.display().to_string(),
    };
    crate::error::lift(
        spawn(app, spec, Controls::default(), move |ctx| {
            let summary = f(ctx)?;
            ctx.message(&summary);
            Ok::<_, String>(Done::new(()).with_result(result))
        })
        .map(|id| id.0),
    )
}

/// An export of the window's logs to `path` as a background export job (two at a time); see [`start_file`].
pub(crate) fn export<R: Runtime>(
    app: &AppHandle<R>,
    win: &CurrentWindow,
    title: &str,
    path: &std::path::Path,
    f: impl FnOnce(&dyn imt_core::LogStore) -> anyhow::Result<String> + Send + 'static,
) -> CmdResult<u64> {
    let spec = export_spec(title, Some(win.label()), path);
    let w = win.arc().clone();
    start_file(app, spec, path, move |_| read_store(&w, f).map_err(crate::err))
}

/// Queues `f` as a job nobody waits for (Retry, background starts). Returns its id.
pub(crate) fn spawn<R, T, E>(
    app: &AppHandle<R>,
    spec: JobSpec,
    controls: Controls,
    f: impl FnOnce(&JobCtx) -> Result<Done<T>, E> + Send + 'static,
) -> Result<JobId, String>
where
    R: Runtime,
    T: Send + 'static,
    E: JobFailure + Send + 'static,
{
    let jobs = app.try_state::<Jobs>().ok_or("IMT is not ready yet")?;
    jobs.submit(spec, controls, f).map(|h| h.id).map_err(|r| r.to_string())
}

fn service<R: Runtime>(app: &AppHandle<R>) -> CmdResult<Jobs> {
    crate::error::lift(
        app.try_state::<Jobs>()
            .map(|j| j.inner().clone())
            .ok_or_else(|| "IMT is not ready yet".to_string()),
    )
}

/// The job, when the calling window may act on it (its own or an app-wide one).
fn owned(jobs: &Jobs, win: &CurrentWindow, id: u64) -> CmdResult<model::Job> {
    let job = jobs.get(JobId(id)).ok_or_else(|| {
        crate::error::CmdError::not_found("This job is no longer in the Jobs list: it was cleared, so its result is gone.")
    })?;
    match job.window() {
        Some(w) if w != win.label() => Err("this job belongs to another window".into()),
        _ => Ok(job),
    }
}

/// A window closed: its jobs stop and its history goes.
pub fn forget_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(jobs) = app.try_state::<Jobs>() {
        jobs.forget_window(label);
    }
}

/// IMT is closing.
pub fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(jobs) = app.try_state::<Jobs>() {
        jobs.shutdown();
    }
}

#[tauri::command]
pub async fn jobs_list<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<JobsSnapshot> {
    Ok(service(&app)?.snapshot(Some(win.label())))
}

/// Async (off the main thread) like the other job commands that change the queue: they notify the
/// tray and the windows.
#[tauri::command]
pub async fn jobs_cancel<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<()> {
    let jobs = service(&app)?;
    owned(&jobs, &win, id)?;
    crate::error::lift(jobs.cancel(JobId(id)))
}

#[tauri::command]
/// Runs a failed, cancelled or interrupted job again; returns the new job (null when the window starts it).
pub async fn jobs_retry<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<Option<u64>> {
    let jobs = service(&app)?;
    owned(&jobs, &win, id)?;
    crate::error::lift(jobs.retry(JobId(id)).map(|j| j.map(|j| j.0)))
}

#[tauri::command]
pub async fn jobs_move<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64, up: bool) -> CmdResult<()> {
    let jobs = service(&app)?;
    owned(&jobs, &win, id)?;
    crate::error::lift(jobs.move_job(JobId(id), up))
}

#[tauri::command]
pub async fn jobs_clear_finished<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<usize> {
    Ok(service(&app)?.clear_finished(Some(win.label())))
}

/// The user viewed a job's result: its unseen mark goes.
#[tauri::command]
pub async fn jobs_mark_seen<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<bool> {
    let jobs = service(&app)?;
    owned(&jobs, &win, id)?;
    Ok(jobs.mark_seen(JobId(id)))
}

/// What View opens. Marks the job seen.
#[tauri::command]
pub async fn jobs_result<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<Option<JobResult>> {
    let jobs = service(&app)?;
    let job = owned(&jobs, &win, id)?;
    jobs.mark_seen(JobId(id));
    Ok(job.result)
}

/// Waits for a job to end (a dialog showing a request's progress while it is open): its state, full result
/// and error. Closing the dialog only stops waiting; the job goes on and tells the user when it is done.
#[tauri::command]
pub async fn jobs_wait<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<service::JobOutcome> {
    let jobs = service(&app)?;
    if let Some(job) = jobs.get(JobId(id)) {
        if job.window().is_some_and(|w| w != win.label()) {
            return Err("this job belongs to another window".into());
        }
    }
    crate::error::lift(
        tauri::async_runtime::spawn_blocking(move || jobs.wait_end(JobId(id)))
            .await
            .map_err(crate::err),
    )
}

/// The dialog that waited for job `id` closed: the job goes on and tells the user when it is done.
#[tauri::command]
pub async fn jobs_unwait<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<()> {
    let jobs = service(&app)?;
    owned(&jobs, &win, id)?;
    jobs.unwait(JobId(id));
    Ok(())
}

/// Shows an export's file in the OS file manager (only the file the job wrote); returns its path.
#[tauri::command]
pub async fn jobs_reveal<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> CmdResult<String> {
    let jobs = service(&app)?;
    let job = owned(&jobs, &win, id)?;
    let Some(JobResult::File { path }) = job.result else {
        return Err("this job did not write a file".into());
    };
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(crate::CmdError::from(format!("{path} no longer exists")));
    }
    jobs.mark_seen(JobId(id));
    crate::opener::start(&crate::opener::reveal(crate::opener::Os::CURRENT, &p))
        .map_err(|e| format!("could not show the file ({e}); it is at {path}"))?;
    Ok(path)
}

/// The window's finished jobs for its project and session (`JobsRecord` v1).
#[tauri::command]
pub async fn jobs_record<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<persist::JobsRecord> {
    Ok(service(&app)?.record(win.label()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredJobs {
    pub snapshot: JobsSnapshot,
    pub warning: Option<String>,
}

/// Replaces the window's history with a saved one; unfinished jobs come back as interrupted.
#[tauri::command]
pub async fn jobs_restore<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, record: Option<Value>) -> CmdResult<RestoredJobs> {
    let jobs = service(&app)?;
    let warning = jobs.restore(win.label(), record.as_ref());
    Ok(RestoredJobs {
        snapshot: jobs.snapshot(Some(win.label())),
        warning,
    })
}

#[cfg(test)]
mod tests;
