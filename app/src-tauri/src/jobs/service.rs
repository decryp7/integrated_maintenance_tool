//! The jobs service: the [`JobQueue`] behind a lock, plus what the queue cannot hold: the work of queued
//! jobs, the stop actions of running ones, progress probes, the threads that run jobs, and listeners.
//! No Tauri here (the glue in `super` wires it to the app), and the clock is injected, so tests drive it
//! with fakes.
//!
//! Every job runs on its own thread once the queue lets it start; a panic in its work fails only that
//! job (with the reason), and the next one starts. A ticker samples progress probes and tells listeners
//! about changes at most every [`TICK`], so producers never flood the UI.

use super::model::{Job, JobId, JobKind, JobProgress, JobResult, JobSpec, JobState};
use super::persist::JobsRecord;
use super::queue::{Cancel, Finished, JobEnd, JobInfo, JobQueue, JobsSnapshot, QueueError, Refusal};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How often progress is sampled and listeners told while jobs are active.
pub const TICK: Duration = Duration::from_millis(400);

/// Epoch milliseconds; injected so tests control time.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// Stops a running job (sets its producer's cancel flag, kills a process, ...). Must return quickly.
pub type CancelFn = Arc<dyn Fn() + Send + Sync>;
/// Reads a running job's progress (for producers that keep their own, like imports).
pub type ProbeFn = Arc<dyn Fn() -> Option<JobProgress> + Send + Sync>;
/// Decides whether a kind may start jobs now: `Some(reason)` pauses its queue (e.g. the AI is quarantined).
pub type GateFn = Arc<dyn Fn() -> Option<String> + Send + Sync>;
/// Runs a failed or interrupted job again from its saved request; returns the new job (none when the
/// UI starts it, e.g. an import handed back to the window).
pub type RetryFn = Arc<dyn Fn(&Job) -> Result<Option<JobId>, String> + Send + Sync>;

type Work = Box<dyn FnOnce(&JobCtx) -> JobEnd + Send>;

/// What listeners hear.
pub enum JobsEvent<'a> {
    /// Something changed (submitted, started, progress, moved, cleared, ...).
    Changed,
    /// A job ended (also followed by `Changed`).
    Finished(&'a Finished),
    /// A gate paused a kind's queue (followed by `Changed`).
    Paused { kind: JobKind, reason: &'a str },
}

type Listener = Arc<dyn Fn(&JobsEvent<'_>) + Send + Sync>;

/// Optional hooks of a job.
#[derive(Default, Clone)]
pub struct Controls {
    pub cancel: Option<CancelFn>,
    pub probe: Option<ProbeFn>,
}

impl Controls {
    pub fn cancel(f: impl Fn() + Send + Sync + 'static) -> Controls {
        Controls {
            cancel: Some(Arc::new(f)),
            probe: None,
        }
    }

    pub fn with_probe(mut self, f: impl Fn() -> Option<JobProgress> + Send + Sync + 'static) -> Controls {
        self.probe = Some(Arc::new(f));
        self
    }
}

/// What a job's work sees: its id, its cancel flag, and a way to report progress.
pub struct JobCtx {
    pub id: JobId,
    cancel: Arc<AtomicBool>,
    jobs: Option<Jobs>,
}

impl JobCtx {
    /// A context outside the service (the app runs without it, e.g. in some tests).
    pub fn detached() -> JobCtx {
        JobCtx {
            id: JobId(0),
            cancel: Arc::new(AtomicBool::new(false)),
            jobs: None,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The flag Cancel sets, for engines that poll their own.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn progress(&self, p: JobProgress) {
        if let Some(j) = &self.jobs {
            j.lock().queue.report(self.id, Some(p), None);
        }
    }

    pub fn message(&self, m: &str) {
        if let Some(j) = &self.jobs {
            j.lock().queue.report(self.id, None, Some(m));
        }
    }
}

/// What a job's work returns on success.
pub struct Done<T> {
    pub value: T,
    pub result: Option<JobResult>,
    /// The work stopped early on request (the value says how far it got).
    pub cancelled: bool,
}

impl<T> Done<T> {
    pub fn new(value: T) -> Done<T> {
        Done {
            value,
            result: None,
            cancelled: false,
        }
    }

    pub fn with_result(mut self, r: JobResult) -> Done<T> {
        self.result = Some(r);
        self
    }

    pub fn cancelled_if(mut self, cancelled: bool) -> Done<T> {
        self.cancelled = cancelled;
        self
    }
}

/// An error of a job's work. `cancelled` errors end the job as cancelled instead of failed.
pub trait JobFailure: std::fmt::Display {
    fn cancelled(&self) -> bool {
        false
    }
}

impl JobFailure for String {}
impl JobFailure for anyhow::Error {}

/// Why a job's work produced no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    Refused(String),
    Failed(String),
    Cancelled,
    Interrupted(String),
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stopped::Refused(m) | Stopped::Failed(m) | Stopped::Interrupted(m) => f.write_str(m),
            Stopped::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl From<Stopped> for String {
    fn from(s: Stopped) -> String {
        s.to_string()
    }
}

/// The value of a submitted job, once it ran.
pub struct JobHandle<T, E> {
    pub id: JobId,
    rx: mpsc::Receiver<Result<T, E>>,
    jobs: Jobs,
}

impl<T, E> JobHandle<T, E> {
    /// Blocks until the job ended. A job cancelled while queued (or dropped at exit) gives `Stopped`.
    pub fn wait(self) -> Result<Result<T, E>, Stopped> {
        if let Ok(r) = self.rx.recv() {
            return Ok(r);
        }
        // The work is gone without a value: wait (briefly) for the job to be recorded as ended.
        let mut job = self.jobs.get(self.id);
        for _ in 0..200 {
            if job.as_ref().is_none_or(|j| j.state().is_finished()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            job = self.jobs.get(self.id);
        }
        Err(match job {
            Some(j) if j.state() == JobState::Interrupted => Stopped::Interrupted("IMT is closing".into()),
            Some(j) if j.state() == JobState::Failed => Stopped::Failed(j.error.unwrap_or_default()),
            _ => Stopped::Cancelled,
        })
    }
}

struct Control {
    work: Option<Work>,
    cancel_flag: Arc<AtomicBool>,
    cancel: Option<CancelFn>,
    probe: Option<ProbeFn>,
}

struct Inner {
    queue: JobQueue,
    controls: HashMap<JobId, Control>,
}

struct Shared {
    inner: Mutex<Inner>,
    clock: Arc<dyn Clock>,
    listeners: Mutex<Vec<Listener>>,
    gates: Mutex<HashMap<JobKind, GateFn>>,
    retry: Mutex<HashMap<JobKind, RetryFn>>,
    ticking: AtomicBool,
    tick: Duration,
}

/// The service. Cheap to clone (one shared state); managed as Tauri state.
#[derive(Clone)]
pub struct Jobs(Arc<Shared>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn log_state(event: &str, job: &Job, now: i64) {
    let (id, kind, ms) = (job.id.to_string(), job.kind.as_str(), job.run_ms(now).to_string());
    crate::activity::domain_event(event, None, &[("Job", &id), ("Kind", kind), ("DurationMs", &ms)]);
}

impl Jobs {
    pub fn new(clock: Arc<dyn Clock>) -> Jobs {
        Jobs::with_tick(clock, TICK)
    }

    pub fn with_tick(clock: Arc<dyn Clock>, tick: Duration) -> Jobs {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Jobs(Arc::new(Shared {
            inner: Mutex::new(Inner {
                queue: JobQueue::new(cores),
                controls: HashMap::new(),
            }),
            clock,
            listeners: Mutex::new(Vec::new()),
            gates: Mutex::new(HashMap::new()),
            retry: Mutex::new(HashMap::new()),
            ticking: AtomicBool::new(false),
            tick,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.0.inner)
    }

    fn now(&self) -> i64 {
        self.0.clock.now_ms()
    }

    pub fn subscribe(&self, f: impl Fn(&JobsEvent<'_>) + Send + Sync + 'static) {
        lock(&self.0.listeners).push(Arc::new(f));
    }

    /// Lets `gate` pause and resume `kind`'s queue (checked before jobs of that kind start).
    pub fn set_gate(&self, kind: JobKind, gate: impl Fn() -> Option<String> + Send + Sync + 'static) {
        lock(&self.0.gates).insert(kind, Arc::new(gate));
    }

    /// How Retry runs a job of `kind` again.
    pub fn set_retry(&self, kind: JobKind, f: impl Fn(&Job) -> Result<Option<JobId>, String> + Send + Sync + 'static) {
        lock(&self.0.retry).insert(kind, Arc::new(f));
    }

    fn emit(&self, event: &JobsEvent<'_>) {
        let listeners = lock(&self.0.listeners).clone();
        for l in listeners {
            // A failing listener (the UI is gone) must not break the producer.
            if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l(event))) {
                log::error!("A jobs listener crashed: {}", imt_core::guard::panic_message(&*p));
            }
        }
    }

    fn finished(&self, f: &Finished) {
        log_state(
            &format!("job.{}", state_name(f.job.state())),
            &f.job,
            f.job.finished_at.unwrap_or(0),
        );
        self.emit(&JobsEvent::Finished(f));
    }

    /// Queues `work`; it runs on its own thread when its kind has a free slot.
    pub fn submit_work(
        &self,
        spec: JobSpec,
        controls: Controls,
        work: impl FnOnce(&JobCtx) -> JobEnd + Send + 'static,
    ) -> Result<JobId, Refusal> {
        let now = self.now();
        let id = {
            let mut inner = self.lock();
            let id = inner.queue.submit(spec, now)?;
            inner.controls.insert(
                id,
                Control {
                    work: Some(Box::new(work)),
                    cancel_flag: Arc::new(AtomicBool::new(false)),
                    cancel: controls.cancel,
                    probe: controls.probe,
                },
            );
            if let Some(j) = inner.queue.get(id) {
                log_state("job.queued", j, now);
            }
            id
        };
        self.emit(&JobsEvent::Changed);
        self.pump();
        self.ensure_ticker();
        Ok(id)
    }

    /// Queues `f` and hands its value to the returned handle. Errors fail the job with their text
    /// (cancelled ones cancel it); `Done::result` is what View opens.
    pub fn submit<T, E>(
        &self,
        spec: JobSpec,
        controls: Controls,
        f: impl FnOnce(&JobCtx) -> Result<Done<T>, E> + Send + 'static,
    ) -> Result<JobHandle<T, E>, Refusal>
    where
        T: Send + 'static,
        E: JobFailure + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let id = self.submit_work(spec, controls, move |ctx| {
            let (end, value) = match f(ctx) {
                Ok(done) => {
                    let end = if done.cancelled {
                        JobEnd::Cancelled
                    } else {
                        JobEnd::Done(done.result)
                    };
                    (end, Ok(done.value))
                }
                Err(e) if e.cancelled() || ctx.is_cancelled() => (JobEnd::Cancelled, Err(e)),
                Err(e) => (JobEnd::Failed(e.to_string()), Err(e)),
            };
            // The waiter may be gone (a dialog closed): the job still ends normally.
            let _ = tx.send(value);
            end
        })?;
        Ok(JobHandle {
            id,
            rx,
            jobs: self.clone(),
        })
    }

    /// Starts every job the queue allows now (after asking the gates whether their kinds may run).
    pub fn pump(&self) {
        let gates: Vec<(JobKind, GateFn)> = lock(&self.0.gates).iter().map(|(k, g)| (*k, g.clone())).collect();
        // Gates run outside the lock: they may ask other services (the AI's health).
        let verdicts: Vec<(JobKind, Option<String>)> = gates.into_iter().map(|(k, g)| (k, guarded_gate(&g))).collect();
        let now = self.now();
        let mut paused = Vec::new();
        let (start, changed) = {
            let mut inner = self.lock();
            let mut changed = false;
            for (kind, verdict) in verdicts {
                match verdict {
                    Some(reason) if inner.queue.paused(kind) != Some(reason.as_str()) => {
                        log::warn!(Kind = kind.as_str(); "Job queue paused: {reason}");
                        inner.queue.pause(kind, &reason);
                        paused.push((kind, reason));
                        changed = true;
                    }
                    None if inner.queue.resume(kind) => {
                        log::info!(Kind = kind.as_str(); "Job queue resumed");
                        changed = true;
                    }
                    _ => {}
                }
            }
            let mut start = Vec::new();
            for id in inner.queue.startable() {
                if inner.queue.start(id, now).is_err() {
                    continue;
                }
                if let Some(j) = inner.queue.get(id) {
                    log_state("job.started", j, now);
                }
                let control = inner.controls.get_mut(&id);
                if let Some((work, flag)) = control.and_then(|c| c.work.take().map(|w| (w, c.cancel_flag.clone()))) {
                    start.push((id, work, flag));
                }
            }
            let changed = changed || !start.is_empty();
            (start, changed)
        };
        for (id, work, flag) in start {
            self.spawn(id, work, flag);
        }
        for (kind, reason) in &paused {
            self.emit(&JobsEvent::Paused { kind: *kind, reason });
        }
        if changed {
            self.emit(&JobsEvent::Changed);
        }
    }

    fn spawn(&self, id: JobId, work: Work, cancel: Arc<AtomicBool>) {
        let jobs = self.clone();
        let run = move || {
            let ctx = JobCtx {
                id,
                cancel,
                jobs: Some(jobs.clone()),
            };
            let end = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctx))) {
                Ok(JobEnd::Failed(_)) if ctx.is_cancelled() => JobEnd::Cancelled,
                Ok(end) => end,
                Err(p) => {
                    let message = imt_core::guard::panic_message(&*p);
                    log::error!(Job = id.0; "A job crashed: {message}");
                    JobEnd::Failed(format!("the job stopped unexpectedly: {message}"))
                }
            };
            jobs.complete(id, end);
        };
        if let Err(e) = std::thread::Builder::new().name(format!("imt-job-{id}")).spawn(run) {
            // No thread: the job fails, the queue goes on.
            self.complete(id, JobEnd::Failed(format!("cannot start the job: {e}")));
        }
    }

    fn complete(&self, id: JobId, end: JobEnd) {
        let now = self.now();
        let finished = {
            let mut inner = self.lock();
            inner.controls.remove(&id);
            inner.queue.finish(id, end, now)
        };
        match finished {
            Ok(f) => {
                self.finished(&f);
                self.emit(&JobsEvent::Changed);
            }
            Err(e) => log::warn!(Job = id.0; "A job ended twice: {e}"),
        }
        self.pump();
    }

    fn ensure_ticker(&self) {
        if self.0.ticking.swap(true, Ordering::SeqCst) {
            return;
        }
        let jobs = self.clone();
        let spawned = std::thread::Builder::new().name("imt-jobs-ticker".into()).spawn(move || loop {
            std::thread::sleep(jobs.0.tick);
            let probes: Vec<(JobId, ProbeFn)> = {
                let inner = jobs.lock();
                if !inner.queue.has_active() {
                    jobs.0.ticking.store(false, Ordering::SeqCst);
                    break;
                }
                inner
                    .controls
                    .iter()
                    .filter(|(id, _)| inner.queue.get(**id).is_some_and(|j| j.state() == JobState::Running))
                    .filter_map(|(id, c)| c.probe.clone().map(|p| (*id, p)))
                    .collect()
            };
            for (id, probe) in probes {
                let sample = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe())).ok().flatten();
                if let Some(p) = sample {
                    jobs.lock().queue.report(id, Some(p), None);
                }
            }
            jobs.emit(&JobsEvent::Changed);
        });
        if spawned.is_err() {
            self.0.ticking.store(false, Ordering::SeqCst);
        }
    }

    /// Cancels a job: a queued one at once, a running one through its stop action.
    pub fn cancel(&self, id: JobId) -> Result<(), String> {
        let now = self.now();
        let (outcome, stop) = {
            let mut inner = self.lock();
            let outcome = inner.queue.cancel(id, now).map_err(|e| e.to_string())?;
            let stop = match &outcome {
                Cancel::Dequeued(_) => {
                    // Dropping the work drops its result channel: the waiter sees `Stopped::Cancelled`.
                    inner.controls.remove(&id);
                    None
                }
                Cancel::Running => inner.controls.get(&id).map(|c| {
                    c.cancel_flag.store(true, Ordering::Relaxed);
                    c.cancel.clone()
                }),
                _ => None,
            };
            (outcome, stop.flatten())
        };
        match outcome {
            Cancel::Dequeued(f) => {
                self.finished(&f);
                self.emit(&JobsEvent::Changed);
                self.pump();
                Ok(())
            }
            Cancel::Running => {
                if let Some(stop) = stop {
                    if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stop())) {
                        let message = imt_core::guard::panic_message(&*p);
                        log::error!(Job = id.0; "Stopping a job crashed: {message}");
                        return Err(format!("stopping failed: {message}"));
                    }
                }
                Ok(())
            }
            Cancel::NotCancellable => Err("this job cannot be cancelled".into()),
            Cancel::AlreadyFinished => Ok(()),
        }
    }

    /// Moves a queued AI job up or down.
    pub fn move_job(&self, id: JobId, up: bool) -> Result<(), String> {
        self.lock().queue.move_queued(id, up).map_err(|e| e.to_string())?;
        self.emit(&JobsEvent::Changed);
        Ok(())
    }

    /// Runs a failed, cancelled or interrupted job again (a new job; the old one counts as seen).
    pub fn retry(&self, id: JobId) -> Result<Option<JobId>, String> {
        let job = {
            let inner = self.lock();
            let job = inner.queue.get(id).ok_or_else(|| QueueError::Unknown(id).to_string())?;
            if !matches!(job.state(), JobState::Failed | JobState::Interrupted | JobState::Cancelled) || job.request.is_none() {
                return Err("only failed, cancelled or interrupted jobs that know their request can be retried".into());
            }
            job.clone()
        };
        let handler = lock(&self.0.retry)
            .get(&job.kind)
            .cloned()
            .ok_or_else(|| format!("{} jobs cannot be retried", job.kind.as_str()))?;
        let new = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(&job)))
            .map_err(|p| format!("retrying failed: {}", imt_core::guard::panic_message(&*p)))??;
        {
            let mut inner = self.lock();
            if let Some(new) = new {
                inner.queue.set_retry_of(new, id);
            }
            inner.queue.mark_seen(id);
        }
        self.emit(&JobsEvent::Changed);
        Ok(new)
    }

    pub fn mark_seen(&self, id: JobId) -> bool {
        let seen = self.lock().queue.mark_seen(id);
        if seen {
            self.emit(&JobsEvent::Changed);
        }
        seen
    }

    pub fn clear_finished(&self, window: Option<&str>) -> usize {
        let n = self.lock().queue.clear_finished(window);
        self.emit(&JobsEvent::Changed);
        n
    }

    /// Reports a running job's progress (for producers that learn it from elsewhere, like a tool).
    pub fn report(&self, id: JobId, progress: JobProgress) {
        self.lock().queue.report(id, Some(progress), None);
    }

    /// The queued and running jobs matching `pred` (e.g. a window's waiting AI requests).
    pub fn find_active(&self, pred: impl Fn(&Job) -> bool) -> Vec<JobId> {
        let inner = self.lock();
        inner
            .queue
            .active_ids()
            .into_iter()
            .filter(|id| inner.queue.get(*id).is_some_and(&pred))
            .collect()
    }

    /// Running jobs of every window that are visible now (for the tray's background work).
    pub fn running_visible(&self) -> Vec<JobInfo> {
        let now = self.now();
        let inner = self.lock();
        inner
            .queue
            .active_ids()
            .into_iter()
            .filter_map(|id| inner.queue.get(id))
            .filter(|j| j.state() == JobState::Running && super::queue::is_visible(j, now))
            .filter_map(|j| inner.queue.info_of(j.id, now))
            .collect()
    }

    pub fn snapshot(&self, window: Option<&str>) -> JobsSnapshot {
        let now = self.now();
        self.lock().queue.snapshot(window, now)
    }

    #[cfg(test)]
    pub fn info(&self, id: JobId) -> Option<JobInfo> {
        let now = self.now();
        self.lock().queue.info_of(id, now)
    }

    pub fn get(&self, id: JobId) -> Option<Job> {
        self.lock().queue.get(id).cloned()
    }

    #[cfg(test)]
    pub fn state_of(&self, id: JobId) -> Option<JobState> {
        self.lock().queue.get(id).map(Job::state)
    }

    /// The window's history as saved with its project.
    pub fn record(&self, window: &str) -> JobsRecord {
        JobsRecord::new(self.lock().queue.history_of(window))
    }

    /// Replaces the window's history with a saved one; returns why it was not (fully) loaded.
    pub fn restore(&self, window: &str, value: Option<&Value>) -> Option<String> {
        let (jobs, warning) = JobsRecord::from_value(value);
        self.lock().queue.restore(window, jobs);
        self.emit(&JobsEvent::Changed);
        warning
    }

    /// The window closed: its running jobs are stopped, its queued ones and its history dropped.
    pub fn forget_window(&self, window: &str) {
        let ids: Vec<JobId> = {
            let inner = self.lock();
            inner
                .queue
                .active_ids()
                .into_iter()
                .filter(|id| inner.queue.get(*id).and_then(Job::window) == Some(window))
                .collect()
        };
        for id in ids {
            let _ = self.cancel(id);
        }
        self.lock().queue.restore(window, Vec::new());
        self.emit(&JobsEvent::Changed);
    }

    /// IMT is closing: every active job is stopped and ends as interrupted.
    pub fn shutdown(&self) {
        let now = self.now();
        let stops: Vec<CancelFn> = {
            let mut inner = self.lock();
            let stops = inner
                .controls
                .values()
                .filter_map(|c| {
                    c.cancel_flag.store(true, Ordering::Relaxed);
                    c.cancel.clone()
                })
                .collect();
            inner.controls.clear();
            inner.queue.interrupt_all("IMT closed before this job finished", now);
            stops
        };
        for stop in stops {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stop()));
        }
        self.emit(&JobsEvent::Changed);
    }
}

fn guarded_gate(g: &GateFn) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g())).unwrap_or_else(|_| Some("the queue's health check crashed".into()))
}

pub fn state_name(s: JobState) -> &'static str {
    match s {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Done => "done",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Interrupted => "interrupted",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::jobs::model::JobLink;
    use crate::jobs::queue::{ATTENTION_AFTER_MS, VISIBLE_AFTER_MS};
    use std::sync::atomic::AtomicI64;
    use std::time::Instant;

    /// A clock the test moves by hand.
    #[derive(Default)]
    pub(crate) struct ManualClock(pub AtomicI64);

    impl ManualClock {
        pub(crate) fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    pub(crate) fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn service() -> (Jobs, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::default());
        (Jobs::with_tick(clock.clone(), Duration::from_millis(5)), clock)
    }

    fn ai(title: &str) -> JobSpec {
        JobSpec::new(JobKind::Ai, title).window("main")
    }

    /// A job that waits until the test opens its gate.
    fn gated(jobs: &Jobs, spec: JobSpec) -> (JobHandle<u32, String>, mpsc::Sender<Result<u32, String>>) {
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();
        let rx = Mutex::new(rx);
        let h = jobs
            .submit(spec, Controls::default(), move |_ctx| {
                let r = lock(&rx).recv().unwrap_or_else(|_| Err("gone".into()));
                r.map(Done::new)
            })
            .unwrap();
        (h, tx)
    }

    #[test]
    fn jobs_run_one_ai_at_a_time_and_hand_back_their_values() {
        let (jobs, clock) = service();
        let finished = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
        let f = finished.clone();
        jobs.subscribe(move |e| {
            if let JobsEvent::Finished(done) = e {
                lock(&f).push((done.job.title.clone(), done.attention));
            }
        });
        let (first, go1) = gated(&jobs, ai("first"));
        let (second, go2) = gated(&jobs, ai("second"));
        wait_until("the first to run", || jobs.state_of(first.id) == Some(JobState::Running));
        assert_eq!(jobs.state_of(second.id), Some(JobState::Queued));
        assert_eq!(jobs.info(second.id).unwrap().position, Some(1));
        clock.advance(ATTENTION_AFTER_MS);
        go1.send(Ok(1)).unwrap();
        assert_eq!(first.wait(), Ok(Ok(1)));
        wait_until("the second to run", || jobs.state_of(second.id) == Some(JobState::Running));
        go2.send(Err("model failed".into())).unwrap();
        assert_eq!(second.wait(), Ok(Err("model failed".to_string())));
        wait_until("both to be reported", || lock(&finished).len() == 2);
        assert_eq!(*lock(&finished), [("first".to_string(), true), ("second".to_string(), true)]);
        let snap = jobs.snapshot(Some("main"));
        assert_eq!(snap.jobs[0].error.as_deref(), Some("model failed"));
    }

    #[test]
    fn a_crashing_job_fails_alone_and_the_queue_goes_on() {
        let _g = crate::activity::test_capture();
        let (jobs, _) = service();
        let crash: JobHandle<(), String> = jobs
            .submit(ai("crash"), Controls::default(), |_| -> Result<Done<()>, String> {
                panic!("worker died")
            })
            .unwrap();
        let next: JobHandle<u8, String> = jobs.submit(ai("next"), Controls::default(), |_| Ok(Done::new(7))).unwrap();
        assert!(
            matches!(crash.wait(), Err(Stopped::Failed(m)) if m.contains("worker died")),
            "no value: the job failed"
        );
        assert_eq!(next.wait(), Ok(Ok(7)));
        wait_until("the crash to be recorded", || jobs.state_of(JobId(1)) == Some(JobState::Failed));
        let info = jobs.info(JobId(1)).unwrap();
        assert!(info.error.unwrap().contains("stopped unexpectedly: worker died"));
        let logged = crate::activity::captured();
        assert!(
            logged.iter().any(|c| c.message.contains("job.failed")),
            "state changes are in the activity log"
        );
        assert!(logged.iter().any(|c| c.message.contains("job.queued")));
    }

    #[test]
    fn cancel_queued_and_running_jobs() {
        let (jobs, clock) = service();
        let stopped = Arc::new(AtomicBool::new(false));
        let s = stopped.clone();
        let running: JobHandle<(), String> = jobs
            .submit(ai("running"), Controls::cancel(move || s.store(true, Ordering::SeqCst)), |ctx| {
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err("stopped by the user".into())
            })
            .unwrap();
        let (queued, _go) = gated(&jobs, ai("queued"));
        wait_until("running", || jobs.state_of(running.id) == Some(JobState::Running));
        // Both were shown (2 s): cancelled jobs stay in the history.
        clock.advance(VISIBLE_AFTER_MS);
        jobs.cancel(queued.id).unwrap();
        assert_eq!(queued.wait(), Err(Stopped::Cancelled));
        assert_eq!(jobs.state_of(JobId(2)), Some(JobState::Cancelled));
        jobs.cancel(running.id).unwrap();
        assert!(stopped.load(Ordering::SeqCst), "the stop action ran");
        assert!(running.wait().unwrap().is_err());
        wait_until("cancelled", || jobs.state_of(JobId(1)) == Some(JobState::Cancelled));
        assert!(jobs.cancel(JobId(1)).is_ok(), "cancelling a finished job is harmless");
        assert!(jobs.cancel(JobId(42)).unwrap_err().contains("no longer listed"));
    }

    #[test]
    fn a_failing_gate_pauses_the_kind_until_it_recovers() {
        let (jobs, _) = service();
        let quarantined = Arc::new(AtomicBool::new(true));
        let q = quarantined.clone();
        jobs.set_gate(JobKind::Ai, move || {
            q.load(Ordering::SeqCst)
                .then(|| "The AI engine was switched off after repeated crashes".to_string())
        });
        let h: JobHandle<u8, String> = jobs.submit(ai("waits"), Controls::default(), |_| Ok(Done::new(1))).unwrap();
        let other: JobHandle<u8, String> = jobs
            .submit(JobSpec::new(JobKind::Export, "x"), Controls::default(), |_| Ok(Done::new(2)))
            .unwrap();
        assert_eq!(other.wait(), Ok(Ok(2)), "other kinds go on");
        assert_eq!(jobs.state_of(h.id), Some(JobState::Queued));
        assert_eq!(
            jobs.snapshot(None).paused[0].reason,
            "The AI engine was switched off after repeated crashes"
        );
        quarantined.store(false, Ordering::SeqCst);
        jobs.pump(); // Reset AI
        assert_eq!(h.wait(), Ok(Ok(1)));
        assert!(jobs.snapshot(None).paused.is_empty());
        // A gate that panics pauses rather than letting jobs run unchecked.
        jobs.set_gate(JobKind::Ai, || panic!("health check bug"));
        let (later, _go) = gated(&jobs, ai("later"));
        assert_eq!(jobs.state_of(later.id), Some(JobState::Queued));
        assert!(jobs.snapshot(None).paused[0].reason.contains("crashed"));
    }

    #[test]
    fn retry_runs_the_saved_request_again_and_links_the_jobs() {
        let (jobs, clock) = service();
        let j2 = jobs.clone();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        jobs.set_retry(JobKind::Ai, move |job| {
            let req = job.request.clone().unwrap();
            let spec = JobSpec::new(JobKind::Ai, job.title.clone()).window("main").retry_with(req);
            let g = gate.clone();
            j2.submit_work(spec, Controls::default(), move |_| {
                let _ = lock(&g).recv();
                JobEnd::Done(None)
            })
            .map(Some)
            .map_err(|e| e.to_string())
        });
        let failing: JobHandle<(), String> = jobs
            .submit(
                ai("explain").retry_with(serde_json::json!({"op": "explain"})),
                Controls::default(),
                |_| Err("busy".to_string()),
            )
            .unwrap();
        let id = failing.id;
        assert!(failing.wait().unwrap().is_err());
        wait_until("failed", || jobs.state_of(id) == Some(JobState::Failed));
        let new = jobs.retry(id).unwrap().unwrap();
        assert_eq!(jobs.info(new).unwrap().retry_of, Some(id));
        assert!(!jobs.info(id).unwrap().unseen, "the failure was dealt with");
        clock.advance(VISIBLE_AFTER_MS);
        release.send(()).unwrap();
        wait_until("the retry to finish", || jobs.state_of(new) == Some(JobState::Done));
        assert!(jobs.retry(new).unwrap_err().contains("only failed"));
        assert!(jobs.retry(JobId(999)).unwrap_err().contains("no longer listed"));
        let plain: JobHandle<(), String> = jobs
            .submit(
                JobSpec::new(JobKind::Export, "x").retry_with(serde_json::json!({})),
                Controls::default(),
                |_| Err("disk".to_string()),
            )
            .unwrap();
        let pid = plain.id;
        let _ = plain.wait();
        wait_until("failed", || jobs.state_of(pid) == Some(JobState::Failed));
        assert!(jobs.retry(pid).unwrap_err().contains("cannot be retried"));
    }

    #[test]
    fn probes_and_reports_reach_the_snapshot_through_the_ticker() {
        let (jobs, clock) = service();
        let changes = Arc::new(AtomicI64::new(0));
        let c = changes.clone();
        jobs.subscribe(move |e| {
            if matches!(e, JobsEvent::Changed) {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });
        let (go, rx) = mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        let h: JobHandle<(), String> = jobs
            .submit(
                JobSpec::new(JobKind::Import, "open")
                    .window("main")
                    .link(JobLink::Path { path: "/logs".into() }),
                Controls::default().with_probe(|| Some(JobProgress::counts(3, 4))),
                move |ctx| {
                    ctx.message("reading app.log");
                    let _ = lock(&rx).recv();
                    Ok(Done::new(()).with_result(JobResult::Workspace))
                },
            )
            .unwrap();
        clock.advance(VISIBLE_AFTER_MS);
        wait_until("the probe", || jobs.info(h.id).and_then(|i| i.progress.done) == Some(3));
        assert_eq!(jobs.info(h.id).unwrap().message.as_deref(), Some("reading app.log"));
        let before = changes.load(Ordering::SeqCst);
        wait_until("ticks", || changes.load(Ordering::SeqCst) > before + 1);
        go.send(()).unwrap();
        assert_eq!(h.wait(), Ok(Ok(())));
        wait_until("done", || jobs.state_of(JobId(1)) == Some(JobState::Done));
        assert_eq!(jobs.info(JobId(1)).unwrap().result_type, Some("workspace"));
        // The ticker stops when nothing is active.
        wait_until("the ticker to stop", || !jobs.0.ticking.load(Ordering::SeqCst));
    }

    #[test]
    fn records_restore_windows_and_shutdown() {
        let (jobs, clock) = service();
        let quick: JobHandle<(), String> = jobs.submit(ai("kept"), Controls::default(), |_| Ok(Done::new(()))).unwrap();
        let _ = quick.wait();
        wait_until("kept or dropped", || jobs.state_of(JobId(1)).is_none_or(|s| s.is_finished()));
        // One export runs (until cancelled), another waits for a slot... after a third that runs too.
        let running: JobHandle<(), String> = jobs
            .submit(
                JobSpec::new(JobKind::Export, "running").window("main"),
                Controls::default(),
                |ctx| {
                    while !ctx.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err("stopped".into())
                },
            )
            .unwrap();
        let (_other, _go) = gated(&jobs, JobSpec::new(JobKind::Export, "other").window("main"));
        let (pending, _go2) = gated(&jobs, JobSpec::new(JobKind::Export, "export").window("main"));
        wait_until("two running", || jobs.state_of(running.id) == Some(JobState::Running));
        assert_eq!(jobs.state_of(pending.id), Some(JobState::Queued));
        clock.advance(VISIBLE_AFTER_MS);
        let saved = serde_json::json!({"version": 1, "jobs": [{
            "id": 5, "kind": "ai", "title": "summarise", "source": {"window": "main"}, "state": "running",
            "queuedAt": 1, "startedAt": 2, "cancellable": true, "request": {"op": "summarize"}
        }]});
        assert_eq!(jobs.restore("main", Some(&saved)), None);
        let snap = jobs.snapshot(Some("main"));
        let restored = snap.jobs.iter().find(|j| j.title == "summarise").unwrap();
        assert_eq!(restored.state, JobState::Interrupted);
        assert_eq!(jobs.record("main").jobs.len(), 1);
        assert!(jobs.restore("main", Some(&serde_json::json!("junk"))).unwrap().contains("damaged"));
        assert!(jobs.record("main").jobs.is_empty(), "a damaged record restores an empty history");
        jobs.shutdown();
        assert_eq!(
            pending.wait(),
            Err(Stopped::Interrupted("IMT is closing".into())),
            "the waiting one never runs"
        );
        assert_eq!(running.wait(), Ok(Err("stopped".to_string())), "the running one was told to stop");
        jobs.forget_window("main");
        assert!(jobs.snapshot(Some("main")).jobs.is_empty());
        assert!(!jobs.mark_seen(JobId(1)));
        assert_eq!(jobs.clear_finished(Some("main")), 0);
        assert!(jobs.move_job(JobId(1), true).is_err());
        assert!(jobs.get(JobId(99)).is_none());
    }

    #[test]
    fn listeners_that_panic_do_not_break_producers() {
        let _g = crate::activity::test_capture();
        let (jobs, _) = service();
        jobs.subscribe(|_| panic!("listener bug"));
        let h: JobHandle<u8, String> = jobs.submit(ai("x"), Controls::default(), |_| Ok(Done::new(3))).unwrap();
        assert_eq!(h.wait(), Ok(Ok(3)));
        assert!(crate::activity::captured().iter().any(|c| c.message.contains("listener crashed")));
        assert_eq!(Stopped::Refused("full".into()).to_string(), "full");
        assert_eq!(String::from(Stopped::Cancelled), "cancelled");
        let ctx = JobCtx::detached();
        ctx.progress(JobProgress::fraction(0.5));
        ctx.message("nothing happens");
        assert!(!ctx.is_cancelled() && !ctx.cancel_flag().load(Ordering::SeqCst));
        assert_eq!(state_name(JobState::Interrupted), "interrupted");
    }
}
