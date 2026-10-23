//! The job queue: pure state, no threads and no clock (every call takes `now`, epoch ms), so limits,
//! order, cancellation, pausing, restoring and the visibility rules are unit-tested directly.
//!
//! - One FIFO queue per [`JobKind`]; at most [`JobKind::limit`] run at once (imports: per window).
//! - At most [`QUEUE_CAP`] jobs wait per kind; more are refused with a message.
//! - Queued AI jobs can be moved up and down.
//! - A paused kind (the AI after its worker was quarantined) keeps its queue but starts nothing.
//! - Finished jobs go to the history (the last [`HISTORY_CAP`]); quick ones that never showed are dropped.

use super::model::{IllegalTransition, Job, JobEvent, JobId, JobKind, JobLink, JobProgress, JobResult, JobSource, JobSpec, JobState};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

/// A job appears in the pane once it has existed this long: quick work never flickers in.
pub const VISIBLE_AFTER_MS: i64 = 2_000;
/// A job that ran this long tells the user when it is done (toast, notification centre, OS notification).
pub const ATTENTION_AFTER_MS: i64 = 10_000;
/// Jobs waiting per kind.
pub const QUEUE_CAP: usize = 20;
/// Finished jobs kept (and saved with the project).
pub const HISTORY_CAP: usize = 100;
const MAX_TITLE: usize = 120;
const MAX_MESSAGE: usize = 500;

/// How a running job ended.
#[derive(Debug, Clone, PartialEq)]
pub enum JobEnd {
    Done(Option<JobResult>),
    Failed(String),
    Cancelled,
    Interrupted(String),
}

/// Why a submit was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    QueueFull { kind: JobKind, cap: usize },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::QueueFull { kind, cap } => write!(
                f,
                "{cap} {} jobs are already waiting; wait for some to finish or cancel one in the Jobs pane",
                kind.as_str()
            ),
        }
    }
}

/// Something a queue call could not do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    Unknown(JobId),
    Illegal(IllegalTransition),
    /// The job cannot be moved (not a queued AI job, or already first/last).
    NotMovable,
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::Unknown(id) => write!(f, "job {id} is no longer listed"),
            QueueError::Illegal(t) => t.fmt(f),
            QueueError::NotMovable => f.write_str("only waiting AI jobs can be moved"),
        }
    }
}

impl From<IllegalTransition> for QueueError {
    fn from(t: IllegalTransition) -> Self {
        QueueError::Illegal(t)
    }
}

/// What a cancel request did.
#[derive(Debug, Clone, PartialEq)]
pub enum Cancel {
    /// The job was waiting and is now cancelled (its work never runs).
    Dequeued(Box<Finished>),
    /// The job runs: its producer must stop it; it ends as cancelled when its work returns.
    Running,
    NotCancellable,
    AlreadyFinished,
}

/// A job that just ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub job: Job,
    /// Tell the user (toast, notification centre, OS notification).
    pub attention: bool,
    /// Kept in the history (and shown); quick jobs are dropped.
    pub kept: bool,
}

/// A paused queue, for the pane ("AI paused: … Reset AI").
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedKind {
    pub kind: JobKind,
    pub reason: String,
}

/// A job as the pane shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: JobId,
    pub kind: JobKind,
    pub title: String,
    pub source: JobSource,
    pub state: JobState,
    pub progress: JobProgress,
    pub message: Option<String>,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub elapsed_ms: i64,
    pub eta_ms: Option<u64>,
    pub cancellable: bool,
    pub retryable: bool,
    /// Can be moved up/down (a queued AI job).
    pub movable: bool,
    /// 1-based place in its kind's queue while queued.
    pub position: Option<usize>,
    /// The kind of result View opens (`narrative`, `file`, ...).
    pub result_type: Option<&'static str>,
    pub error: Option<String>,
    /// Finished, needed attention, and the user has not viewed it yet.
    pub unseen: bool,
    pub retry_of: Option<JobId>,
}

/// Everything the pane needs: the visible jobs and the paused queues.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsSnapshot {
    pub jobs: Vec<JobInfo>,
    pub paused: Vec<PausedKind>,
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Whether a finished job asks for attention: a success that took a while, or a failure the user cares
/// about (a quick analysis failure is shown by its own view).
pub fn needs_attention(job: &Job) -> bool {
    match job.state() {
        // A research answer is always news ("Research finished · View answer").
        // So is an AI answer started in the background (explain, summarise, root causes): nobody waits for it.
        JobState::Done => {
            job.kind == JobKind::Research
                || matches!(
                    job.result,
                    Some(
                        JobResult::Narrative { .. }
                            | JobResult::Proposal { .. }
                            | JobResult::File { .. }
                            | JobResult::FolderSurvey { .. }
                            | JobResult::Storage { .. }
                            | JobResult::RelatedLogs { .. }
                    )
                )
                || job.run_ms(i64::MIN) >= ATTENTION_AFTER_MS
                || job.age_ms(i64::MIN) >= ATTENTION_AFTER_MS
        }
        JobState::Failed => job.kind != JobKind::Analysis || job.age_ms(i64::MIN) >= VISIBLE_AFTER_MS,
        _ => false,
    }
}

/// Whether a job shows in the pane at `now`.
pub fn is_visible(job: &Job, now: i64) -> bool {
    job.age_ms(now) >= VISIBLE_AFTER_MS || (job.state() == JobState::Failed && needs_attention(job))
}

fn result_type(r: &JobResult) -> &'static str {
    match r {
        JobResult::Narrative { .. } => "narrative",
        JobResult::Proposal { .. } => "proposal",
        JobResult::Chat { .. } => "chat",
        JobResult::File { .. } => "file",
        JobResult::Value { .. } => "value",
        JobResult::Workspace { .. } => "workspace",
        JobResult::View { .. } => "view",
        JobResult::Model { .. } => "model",
        JobResult::FolderSurvey { .. } => "folderSurvey",
        JobResult::Storage { .. } => "storage",
        JobResult::RelatedLogs { .. } => "relatedLogs",
    }
}

/// The time left: the producer's estimate, else extrapolated from the fraction done.
fn eta(job: &Job, now: i64) -> Option<u64> {
    if job.state() != JobState::Running {
        return None;
    }
    if let Some(e) = job.progress.eta_ms {
        return Some(e);
    }
    let f = f64::from(job.progress.fraction?);
    if !(0.01..1.0).contains(&f) {
        return None;
    }
    let run = job.run_ms(now) as f64;
    Some((run * (1.0 - f) / f).round() as u64)
}

#[derive(Debug, Clone)]
pub struct JobQueue {
    cores: usize,
    /// Queued and running jobs, in submission order (each kind's FIFO is this order filtered by kind).
    active: Vec<Job>,
    /// Finished jobs, oldest first.
    history: VecDeque<Job>,
    next_id: u64,
    paused: BTreeMap<JobKind, String>,
}

impl JobQueue {
    pub fn new(cores: usize) -> JobQueue {
        JobQueue {
            cores,
            active: Vec::new(),
            history: VecDeque::new(),
            next_id: 1,
            paused: BTreeMap::new(),
        }
    }

    fn take_id(&mut self) -> JobId {
        let id = JobId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Queues a job; refused when its kind already has [`QUEUE_CAP`] waiting.
    pub fn submit(&mut self, spec: JobSpec, now: i64) -> Result<JobId, Refusal> {
        let waiting = self
            .active
            .iter()
            .filter(|j| j.kind == spec.kind && j.state() == JobState::Queued)
            .count();
        if waiting >= QUEUE_CAP {
            return Err(Refusal::QueueFull {
                kind: spec.kind,
                cap: QUEUE_CAP,
            });
        }
        let id = self.take_id();
        let mut source = spec.source;
        if let Some(JobLink::Query { ql }) = &mut source.link {
            *ql = clean(ql, 2000);
        }
        self.active.push(Job {
            id,
            kind: spec.kind,
            title: clean(&spec.title, MAX_TITLE),
            source,
            state: JobState::Queued,
            progress: JobProgress::default(),
            message: None,
            queued_at: now,
            started_at: None,
            finished_at: None,
            cancellable: spec.cancellable,
            result: None,
            error: None,
            seen: false,
            request: spec.request,
            retry_of: None,
        });
        Ok(id)
    }

    /// Marks the job as a retry of `of`.
    pub fn set_retry_of(&mut self, id: JobId, of: JobId) {
        if let Some(j) = self.active.iter_mut().find(|j| j.id == id) {
            j.retry_of = Some(of);
        }
    }

    fn running(&self, kind: JobKind, window: Option<&str>) -> usize {
        self.active
            .iter()
            .filter(|j| j.state() == JobState::Running && j.kind == kind && (!kind.per_window() || j.window() == window))
            .count()
    }

    /// The queued jobs that may start now, in order: FIFO per kind, within each kind's limit, none of a
    /// paused kind.
    pub fn startable(&self) -> Vec<JobId> {
        let mut chosen: Vec<(JobKind, Option<&str>)> = Vec::new();
        let mut out = Vec::new();
        for j in self.active.iter().filter(|j| j.state() == JobState::Queued) {
            if self.paused.contains_key(&j.kind) {
                continue;
            }
            let window = if j.kind.per_window() { j.window() } else { None };
            let already = chosen.iter().filter(|(k, w)| *k == j.kind && *w == window).count();
            if self.running(j.kind, window) + already < j.kind.limit(self.cores) {
                chosen.push((j.kind, window));
                out.push(j.id);
            }
        }
        out
    }

    fn active_mut(&mut self, id: JobId) -> Result<&mut Job, QueueError> {
        self.active.iter_mut().find(|j| j.id == id).ok_or(QueueError::Unknown(id))
    }

    pub fn start(&mut self, id: JobId, now: i64) -> Result<(), QueueError> {
        let j = self.active_mut(id)?;
        j.state = j.state.step(JobEvent::Start)?;
        j.started_at = Some(now);
        Ok(())
    }

    /// Updates a running job's progress and message. False for unknown or finished jobs.
    pub fn report(&mut self, id: JobId, progress: Option<JobProgress>, message: Option<&str>) -> bool {
        let Ok(j) = self.active_mut(id) else { return false };
        if let Some(p) = progress {
            j.progress = p.sanitized();
        }
        if let Some(m) = message {
            j.message = Some(clean(m, MAX_MESSAGE)).filter(|m| !m.is_empty());
        }
        true
    }

    /// Ends an active job and moves it to the history (or drops it when it was quick and needs nothing).
    pub fn finish(&mut self, id: JobId, end: JobEnd, now: i64) -> Result<Finished, QueueError> {
        let i = self.active.iter().position(|j| j.id == id).ok_or(QueueError::Unknown(id))?;
        let event = match &end {
            JobEnd::Done(_) => JobEvent::Succeed,
            JobEnd::Failed(_) => JobEvent::Fail,
            JobEnd::Cancelled => JobEvent::Cancel,
            JobEnd::Interrupted(_) => JobEvent::Interrupt,
        };
        let next = self.active[i].state.step(event)?;
        let mut job = self.active.remove(i);
        job.state = next;
        job.finished_at = Some(now.max(job.queued_at));
        match end {
            JobEnd::Done(result) => {
                job.result = result.and_then(JobResult::bounded);
                if job.progress.fraction.is_some() {
                    job.progress.fraction = Some(1.0);
                }
            }
            JobEnd::Failed(reason) | JobEnd::Interrupted(reason) => job.error = Some(clean(&reason, MAX_MESSAGE)),
            JobEnd::Cancelled => {}
        }
        let attention = needs_attention(&job);
        // Nothing to look at: counts as seen.
        job.seen = !attention;
        let kept = is_visible(&job, now) || attention;
        if kept {
            self.keep(job.clone());
        }
        Ok(Finished { job, attention, kept })
    }

    fn keep(&mut self, job: Job) {
        self.history.push_back(job);
        while self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
    }

    /// Cancels a queued job at once; a running one must be stopped by its producer.
    pub fn cancel(&mut self, id: JobId, now: i64) -> Result<Cancel, QueueError> {
        let Some(j) = self.active.iter().find(|j| j.id == id) else {
            return if self.history.iter().any(|j| j.id == id) {
                Ok(Cancel::AlreadyFinished)
            } else {
                Err(QueueError::Unknown(id))
            };
        };
        match j.state() {
            JobState::Queued => Ok(Cancel::Dequeued(Box::new(self.finish(id, JobEnd::Cancelled, now)?))),
            _ if !j.cancellable => Ok(Cancel::NotCancellable),
            _ => Ok(Cancel::Running),
        }
    }

    /// Moves a queued, reorderable job one place up (`up`) or down within its kind's queue.
    pub fn move_queued(&mut self, id: JobId, up: bool) -> Result<(), QueueError> {
        let i = self.active.iter().position(|j| j.id == id).ok_or(QueueError::Unknown(id))?;
        let kind = self.active[i].kind;
        if !kind.reorderable() || self.active[i].state() != JobState::Queued {
            return Err(QueueError::NotMovable);
        }
        let same = |j: &Job| j.kind == kind && j.state() == JobState::Queued;
        let other = if up {
            self.active[..i].iter().rposition(same)
        } else {
            self.active[i + 1..].iter().position(same).map(|p| p + i + 1)
        };
        let o = other.ok_or(QueueError::NotMovable)?;
        self.active.swap(i, o);
        Ok(())
    }

    /// Stops starting jobs of `kind` (its queue is kept).
    pub fn pause(&mut self, kind: JobKind, reason: &str) {
        self.paused.insert(kind, clean(reason, MAX_MESSAGE));
    }

    /// Lets a paused kind start jobs again. False when it was not paused.
    pub fn resume(&mut self, kind: JobKind) -> bool {
        self.paused.remove(&kind).is_some()
    }

    pub fn paused(&self, kind: JobKind) -> Option<&str> {
        self.paused.get(&kind).map(String::as_str)
    }

    /// Ends every queued and running job as interrupted (Debrief is closing). Returns them.
    pub fn interrupt_all(&mut self, reason: &str, now: i64) -> Vec<Finished> {
        let ids: Vec<JobId> = self.active.iter().map(|j| j.id).collect();
        ids.into_iter()
            .filter_map(|id| self.finish(id, JobEnd::Interrupted(reason.to_string()), now).ok())
            .collect()
    }

    /// Replaces the window's history with saved jobs: renumbered, oldest first, bounded; jobs that were
    /// queued or running when Debrief closed come back as interrupted (with Retry when they have a request).
    pub fn restore(&mut self, window: &str, saved: Vec<Job>) {
        self.history.retain(|j| j.window() != Some(window));
        let skip = saved.len().saturating_sub(HISTORY_CAP);
        for mut job in saved.into_iter().skip(skip) {
            job.id = self.take_id();
            job.retry_of = None;
            job.source.window = Some(window.to_string());
            job.title = clean(&job.title, MAX_TITLE);
            if !job.state.is_finished() {
                job.state = JobState::Interrupted;
                job.finished_at = Some(job.started_at.unwrap_or(job.queued_at));
                job.error = Some("Debrief closed before this job finished".into());
                job.seen = false;
            }
            job.result = job.result.and_then(JobResult::bounded);
            self.keep(job);
        }
        let mut sorted: Vec<Job> = self.history.drain(..).collect();
        sorted.sort_by_key(|j| j.finished_at.unwrap_or(j.queued_at));
        self.history = sorted.into();
    }

    /// The window's finished jobs, as saved with its project (oldest first).
    pub fn history_of(&self, window: &str) -> Vec<Job> {
        self.history.iter().filter(|j| j.window() == Some(window)).cloned().collect()
    }

    /// Removes the finished jobs the window shows (its own and app-wide ones). Returns how many.
    pub fn clear_finished(&mut self, window: Option<&str>) -> usize {
        let before = self.history.len();
        self.history.retain(|j| !(j.window().is_none() || j.window() == window));
        before - self.history.len()
    }

    /// The user viewed the job's result. False for an unknown job.
    pub fn mark_seen(&mut self, id: JobId) -> bool {
        match self.history.iter_mut().find(|j| j.id == id) {
            Some(j) => {
                j.seen = true;
                true
            }
            None => false,
        }
    }

    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.active
            .iter()
            .find(|j| j.id == id)
            .or_else(|| self.history.iter().find(|j| j.id == id))
    }

    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn active_ids(&self) -> Vec<JobId> {
        self.active.iter().map(|j| j.id).collect()
    }

    /// 1-based place of a queued job in its kind's queue.
    pub fn position(&self, id: JobId) -> Option<usize> {
        let j = self.active.iter().find(|j| j.id == id && j.state() == JobState::Queued)?;
        let ahead = self
            .active
            .iter()
            .take_while(|o| o.id != id)
            .filter(|o| o.kind == j.kind && o.state() == JobState::Queued && (!j.kind.per_window() || o.window() == j.window()))
            .count();
        Some(ahead + 1)
    }

    fn info(&self, j: &Job, now: i64) -> JobInfo {
        let finished = j.state().is_finished();
        JobInfo {
            id: j.id,
            kind: j.kind,
            title: j.title.clone(),
            source: j.source.clone(),
            state: j.state(),
            progress: j.progress.clone(),
            message: j.message.clone(),
            queued_at: j.queued_at,
            started_at: j.started_at,
            finished_at: j.finished_at,
            elapsed_ms: j.run_ms(now),
            eta_ms: eta(j, now),
            cancellable: j.cancellable && !finished,
            retryable: j.request.is_some() && matches!(j.state(), JobState::Failed | JobState::Interrupted | JobState::Cancelled),
            movable: j.kind.reorderable() && j.state() == JobState::Queued,
            position: self.position(j.id),
            result_type: j.result.as_ref().map(result_type),
            error: j.error.clone(),
            unseen: finished && !j.seen,
            retry_of: j.retry_of,
        }
    }

    /// What a window shows: its jobs and app-wide ones that are visible at `now`; running, then queued
    /// (in queue order), then finished (newest first).
    pub fn snapshot(&self, window: Option<&str>, now: i64) -> JobsSnapshot {
        let mine = |j: &&Job| j.window().is_none() || j.window() == window;
        let mut jobs: Vec<JobInfo> = self
            .active
            .iter()
            .filter(mine)
            .filter(|j| is_visible(j, now))
            .map(|j| self.info(j, now))
            .collect();
        jobs.sort_by_key(|j| j.state != JobState::Running);
        jobs.extend(self.history.iter().rev().filter(mine).map(|j| self.info(j, now)));
        JobsSnapshot {
            jobs,
            paused: self
                .paused
                .iter()
                .map(|(kind, reason)| PausedKind {
                    kind: *kind,
                    reason: reason.clone(),
                })
                .collect(),
        }
    }

    /// The info of one job (visible or not), e.g. for the chat's "queued (position n)".
    pub fn info_of(&self, id: JobId, now: i64) -> Option<JobInfo> {
        self.get(id).map(|j| self.info(j, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ai(title: &str) -> JobSpec {
        JobSpec::new(JobKind::Ai, title).window("main")
    }

    fn states(q: &JobQueue, now: i64) -> Vec<(String, JobState)> {
        q.snapshot(Some("main"), now).jobs.into_iter().map(|j| (j.title, j.state)).collect()
    }

    #[test]
    fn each_kind_runs_within_its_limit_and_imports_per_window() {
        let mut q = JobQueue::new(4);
        let a1 = q.submit(ai("a1"), 0).unwrap();
        let a2 = q.submit(ai("a2"), 0).unwrap();
        let x: Vec<_> = (0..4)
            .map(|i| q.submit(JobSpec::new(JobKind::Analysis, format!("x{i}")), 0).unwrap())
            .collect();
        let e: Vec<_> = (0..3)
            .map(|i| q.submit(JobSpec::new(JobKind::Export, format!("e{i}")), 0).unwrap())
            .collect();
        let i1 = q.submit(JobSpec::new(JobKind::Import, "i1").window("main"), 0).unwrap();
        let i2 = q.submit(JobSpec::new(JobKind::Import, "i2").window("main"), 0).unwrap();
        let i3 = q.submit(JobSpec::new(JobKind::Import, "i3").window("w-2"), 0).unwrap();
        let start = q.startable();
        assert_eq!(
            start,
            vec![a1, x[0], x[1], x[2], e[0], e[1], i1, i3],
            "AI 1, analysis cores-1, export 2, import 1 per window"
        );
        for id in &start {
            q.start(*id, 1).unwrap();
        }
        assert!(q.startable().is_empty());
        q.finish(a1, JobEnd::Done(None), 5).unwrap();
        q.finish(i1, JobEnd::Done(None), 5).unwrap();
        assert_eq!(q.startable(), vec![a2, i2]);
        assert!(!start.contains(&x[3]) && !start.contains(&e[2]));
    }

    #[test]
    fn the_queue_is_capped_per_kind_with_a_clear_message() {
        let mut q = JobQueue::new(2);
        for i in 0..QUEUE_CAP {
            q.submit(ai(&format!("a{i}")), 0).unwrap();
        }
        let refused = q.submit(ai("one too many"), 0).unwrap_err();
        assert_eq!(
            refused,
            Refusal::QueueFull {
                kind: JobKind::Ai,
                cap: QUEUE_CAP
            }
        );
        assert!(refused.to_string().contains("20 ai jobs are already waiting"));
        // Other kinds have their own queue; a running job frees a waiting place.
        q.submit(JobSpec::new(JobKind::Export, "e"), 0).unwrap();
        let first = q.startable()[0];
        q.start(first, 0).unwrap();
        q.submit(ai("now fits"), 0).unwrap();
    }

    #[test]
    fn fifo_per_kind_and_ai_jobs_can_be_reordered() {
        let mut q = JobQueue::new(2);
        let run = q.submit(ai("running"), 0).unwrap();
        q.start(run, 0).unwrap();
        let a = q.submit(ai("a"), 0).unwrap();
        let x = q.submit(JobSpec::new(JobKind::Export, "x"), 0).unwrap();
        let b = q.submit(ai("b"), 0).unwrap();
        let c = q.submit(ai("c"), 0).unwrap();
        assert_eq!((q.position(a), q.position(b), q.position(c)), (Some(1), Some(2), Some(3)));
        q.move_queued(c, true).unwrap();
        assert_eq!((q.position(b), q.position(c)), (Some(3), Some(2)));
        q.move_queued(a, false).unwrap();
        assert_eq!(q.position(c), Some(1));
        assert_eq!(q.move_queued(c, true), Err(QueueError::NotMovable), "already first");
        assert_eq!(q.move_queued(b, false), Err(QueueError::NotMovable), "already last");
        assert_eq!(q.move_queued(x, true), Err(QueueError::NotMovable), "exports keep FIFO");
        assert_eq!(q.move_queued(run, true), Err(QueueError::NotMovable), "running");
        assert_eq!(q.move_queued(JobId(99), true), Err(QueueError::Unknown(JobId(99))));
        assert_eq!(q.position(run), None);
        q.finish(run, JobEnd::Done(None), 1).unwrap();
        assert_eq!(q.startable(), vec![c, x]);
        let snap = q.snapshot(Some("main"), 10_000);
        let movable: Vec<_> = snap.jobs.iter().filter(|j| j.movable).map(|j| j.title.as_str()).collect();
        assert_eq!(movable, ["c", "a", "b"]);
    }

    #[test]
    fn cancelling_queued_and_running_jobs() {
        let mut q = JobQueue::new(2);
        let run = q.submit(ai("run"), 0).unwrap();
        q.start(run, 0).unwrap();
        let wait = q.submit(ai("wait"), 0).unwrap();
        let Cancel::Dequeued(f) = q.cancel(wait, 3_000).unwrap() else {
            panic!("queued jobs end at once")
        };
        assert_eq!(f.job.state(), JobState::Cancelled);
        assert!(f.kept && !f.attention, "it showed (3 s) but needs nothing");
        assert_eq!(q.cancel(run, 3_000).unwrap(), Cancel::Running, "the producer stops it");
        assert_eq!(q.get(run).unwrap().state(), JobState::Running);
        q.finish(run, JobEnd::Cancelled, 3_100).unwrap();
        assert_eq!(q.cancel(run, 3_200).unwrap(), Cancel::AlreadyFinished);
        assert_eq!(q.cancel(JobId(77), 0), Err(QueueError::Unknown(JobId(77))));
        let fixed = q.submit(ai("fixed").not_cancellable(), 0).unwrap();
        q.start(fixed, 0).unwrap();
        assert_eq!(q.cancel(fixed, 0).unwrap(), Cancel::NotCancellable);
        // A finished job cannot finish again.
        assert!(matches!(q.finish(run, JobEnd::Done(None), 0), Err(QueueError::Unknown(_))));
        assert!(matches!(q.finish(wait, JobEnd::Done(None), 0), Err(QueueError::Unknown(_))));
    }

    #[test]
    fn a_failed_job_frees_its_slot_for_the_next() {
        let mut q = JobQueue::new(2);
        let a = q.submit(ai("crashes"), 0).unwrap();
        let b = q.submit(ai("next"), 0).unwrap();
        q.start(a, 0).unwrap();
        let f = q.finish(a, JobEnd::Failed("the AI worker crashed\n(exit 3)".into()), 500).unwrap();
        assert_eq!(f.job.error.as_deref(), Some("the AI worker crashed(exit 3)"));
        assert!(f.attention && f.kept, "failures always ask for attention");
        assert_eq!(q.startable(), vec![b]);
        // Starting a job that is not queued is illegal.
        assert!(matches!(q.start(a, 0), Err(QueueError::Unknown(_))));
        q.start(b, 600).unwrap();
        assert!(matches!(q.start(b, 0), Err(QueueError::Illegal(_))));
    }

    #[test]
    fn a_paused_kind_keeps_its_queue_and_starts_nothing() {
        let mut q = JobQueue::new(4);
        let a = q.submit(ai("a"), 0).unwrap();
        let x = q.submit(JobSpec::new(JobKind::Analysis, "x"), 0).unwrap();
        q.pause(JobKind::Ai, "The AI worker crashed repeatedly");
        assert_eq!(q.startable(), vec![x], "other kinds go on");
        assert_eq!(q.paused(JobKind::Ai), Some("The AI worker crashed repeatedly"));
        let snap = q.snapshot(Some("main"), 0);
        assert_eq!(
            snap.paused,
            vec![PausedKind {
                kind: JobKind::Ai,
                reason: "The AI worker crashed repeatedly".into()
            }]
        );
        assert!(q.resume(JobKind::Ai));
        assert!(!q.resume(JobKind::Ai));
        assert_eq!(q.startable(), vec![a, x]);
    }

    #[test]
    fn visibility_and_attention_thresholds() {
        let mut q = JobQueue::new(2);
        let quick = q.submit(JobSpec::new(JobKind::Analysis, "quick").window("main"), 0).unwrap();
        q.start(quick, 0).unwrap();
        assert!(states(&q, VISIBLE_AFTER_MS - 1).is_empty(), "not shown before 2 s");
        let f = q.finish(quick, JobEnd::Done(None), 1_500).unwrap();
        assert!(!f.kept && !f.attention, "quick work never flickers in");
        let quick_fail = q.submit(JobSpec::new(JobKind::Analysis, "bad query").window("main"), 0).unwrap();
        q.start(quick_fail, 0).unwrap();
        let f = q.finish(quick_fail, JobEnd::Failed("syntax".into()), 100).unwrap();
        assert!(!f.kept && !f.attention, "the view shows a quick analysis failure itself");
        let ai_fail = q.submit(ai("explain"), 0).unwrap();
        q.start(ai_fail, 0).unwrap();
        assert!(q.finish(ai_fail, JobEnd::Failed("no model".into()), 100).unwrap().attention);

        let slow = q.submit(ai("slow"), 10_000).unwrap();
        q.start(slow, 10_000).unwrap();
        assert_eq!(states(&q, 11_999).len(), 1, "only the AI failure");
        assert_eq!(states(&q, 12_000)[0], ("slow".into(), JobState::Running));
        let medium = q.finish(slow, JobEnd::Done(None), 15_000).unwrap();
        assert!(medium.kept && !medium.attention, "5 s: shown, no toast");
        let long = q.submit(ai("long"), 20_000).unwrap();
        q.start(long, 20_000).unwrap();
        let f = q
            .finish(long, JobEnd::Done(Some(JobResult::workspace())), 20_000 + ATTENTION_AFTER_MS)
            .unwrap();
        assert!(f.attention && !f.job.seen);
        let info = q.info_of(long, 40_000).unwrap();
        assert!(info.unseen);
        assert_eq!(info.result_type, Some("workspace"));
        assert!(q.mark_seen(long));
        assert!(!q.info_of(long, 40_000).unwrap().unseen);
        assert!(!q.mark_seen(JobId(999)));
    }

    #[test]
    fn elapsed_and_eta_come_from_the_clock_and_the_progress() {
        let mut q = JobQueue::new(2);
        let id = q.submit(JobSpec::new(JobKind::Export, "report"), 0).unwrap();
        q.start(id, 1_000).unwrap();
        assert!(q.report(id, Some(JobProgress::fraction(0.25)), Some("page 1\u{7}")));
        let info = q.info_of(id, 5_000).unwrap();
        assert_eq!((info.elapsed_ms, info.eta_ms), (4_000, Some(12_000)));
        assert_eq!(info.message.as_deref(), Some("page 1"));
        q.report(
            id,
            Some(JobProgress {
                eta_ms: Some(42),
                ..JobProgress::default()
            }),
            None,
        );
        assert_eq!(q.info_of(id, 5_000).unwrap().eta_ms, Some(42), "the producer's estimate wins");
        q.report(id, Some(JobProgress::fraction(0.0)), None);
        assert_eq!(q.info_of(id, 5_000).unwrap().eta_ms, None, "nothing to extrapolate from");
        q.report(id, Some(JobProgress::fraction(0.5)), None);
        q.finish(id, JobEnd::Done(None), 9_000).unwrap();
        let done = q.info_of(id, 50_000).unwrap();
        assert_eq!((done.elapsed_ms, done.eta_ms, done.progress.fraction), (8_000, None, Some(1.0)));
        assert!(!q.report(id, None, Some("late")), "finished jobs take no more reports");
    }

    #[test]
    fn the_history_is_bounded_and_cleared_per_window() {
        let mut q = JobQueue::new(2);
        for i in 0..HISTORY_CAP + 5 {
            let id = q.submit(JobSpec::new(JobKind::Export, format!("e{i}")).window("main"), 0).unwrap();
            q.start(id, 0).unwrap();
            q.finish(id, JobEnd::Done(None), 3_000).unwrap();
        }
        let app_wide = q.submit(JobSpec::new(JobKind::Download, "model"), 0).unwrap();
        q.start(app_wide, 0).unwrap();
        q.finish(app_wide, JobEnd::Cancelled, 3_000).unwrap();
        let other = q.submit(JobSpec::new(JobKind::Export, "other").window("w-2"), 0).unwrap();
        q.start(other, 0).unwrap();
        q.finish(other, JobEnd::Done(None), 3_000).unwrap();
        assert_eq!(q.history.len(), HISTORY_CAP);
        assert_eq!(q.history_of("main").len(), HISTORY_CAP - 2);
        assert_eq!(q.history_of("main")[0].title, "e7", "the oldest were dropped");
        assert_eq!(q.clear_finished(Some("main")), HISTORY_CAP - 1, "its own and app-wide ones");
        assert_eq!(states(&q, 0).len(), 0);
        assert_eq!(q.history_of("w-2").len(), 1);
    }

    #[test]
    fn restored_jobs_are_renumbered_and_unfinished_ones_interrupted() {
        let mut q = JobQueue::new(2);
        let mut saved = JobQueue::new(2);
        let done = saved
            .submit(ai("explained").retry_with(json!({"op": "explain", "recordId": 3})), 0)
            .unwrap();
        saved.start(done, 0).unwrap();
        saved
            .finish(
                done,
                JobEnd::Done(Some(JobResult::Narrative {
                    narrative: json!({"text": "t"}),
                })),
                20_000,
            )
            .unwrap();
        let running = saved.submit(ai("summarise").retry_with(json!({"op": "summarize"})), 1).unwrap();
        saved.start(running, 2).unwrap();
        saved.submit(ai("waiting"), 3).unwrap();
        let mut jobs = saved.history_of("main");
        jobs.extend(saved.active.clone());

        q.submit(ai("current"), 0).unwrap();
        q.restore("main", jobs);
        let snap = q.snapshot(Some("main"), 100_000);
        let finished: Vec<_> = snap.jobs.iter().filter(|j| j.state.is_finished()).collect();
        assert_eq!(finished.len(), 3);
        let ids: Vec<u64> = finished.iter().map(|j| j.id.0).collect();
        assert!(ids.iter().all(|id| *id > 1), "renumbered after the live job: {ids:?}");
        let summarise = finished.iter().find(|j| j.title == "summarise").unwrap();
        assert_eq!(summarise.state, JobState::Interrupted);
        assert!(summarise.retryable && summarise.unseen);
        assert_eq!(summarise.error.as_deref(), Some("Debrief closed before this job finished"));
        let waiting = finished.iter().find(|j| j.title == "waiting").unwrap();
        assert!(!waiting.retryable, "no request: cannot be retried");
        let explained = finished.iter().find(|j| j.title == "explained").unwrap();
        assert_eq!((explained.state, explained.result_type), (JobState::Done, Some("narrative")));
        // Restoring again replaces, never duplicates.
        let again = q.history_of("main");
        q.restore("main", again);
        assert_eq!(q.history_of("main").len(), 3);
    }

    #[test]
    fn closing_interrupts_everything_active() {
        let mut q = JobQueue::new(2);
        let a = q.submit(ai("a"), 0).unwrap();
        q.start(a, 0).unwrap();
        q.submit(ai("b"), 0).unwrap();
        let ended = q.interrupt_all("Debrief is closing", 5_000);
        assert_eq!(ended.len(), 2);
        assert!(ended.iter().all(|f| f.job.state() == JobState::Interrupted && f.kept));
        assert!(!q.has_active() && q.active_ids().is_empty());
    }

    #[test]
    fn titles_and_queries_are_cleaned() {
        let mut q = JobQueue::new(2);
        let id = q
            .submit(
                JobSpec::new(JobKind::Analysis, format!("a\nb{}", "x".repeat(300))).link(JobLink::Query {
                    ql: "level\u{0}=error".into(),
                }),
                0,
            )
            .unwrap();
        let j = q.get(id).unwrap();
        assert_eq!(j.title.chars().count(), MAX_TITLE);
        assert!(j.title.starts_with("abx"));
        assert_eq!(j.source.link, Some(JobLink::Query { ql: "level=error".into() }));
        q.set_retry_of(id, JobId(1));
        assert_eq!(q.get(id).unwrap().retry_of, Some(JobId(1)));
        assert_eq!(QueueError::Unknown(JobId(3)).to_string(), "job 3 is no longer listed");
        assert!(QueueError::NotMovable.to_string().contains("AI"));
    }
}
