//! The job types: what a job is, which states it passes through and what it leaves behind. Pure data;
//! the queue ([`super::queue`]) is the only place that changes a job's state, through
//! [`JobState::step`], so an illegal transition (a finished job running again, a cancelled job
//! finishing) cannot happen.
//!
//! Wire contract: `app/src/app/jobs/jobs.types.ts` (camelCase JSON). Saved jobs use the same shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A job's id: unique for the app's run (restored jobs are renumbered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a job does. Each kind has its own queue and concurrency limit ([`JobKind::limit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    /// A request to the local model (one at a time: there is one model in memory).
    Ai,
    /// Web research: search, read the result pages, answer with citations (one at a time: one browser).
    Research,
    /// Long queries and analyses (patterns, anomalies, compare, Moment, profiler findings, folder survey).
    Analysis,
    /// Opening or reloading files (one per window: a window has one workspace).
    Import,
    /// Reports, bundles, CSV, visuals, diagnostics archives.
    Export,
    /// AI models and inference runtimes.
    Download,
    /// Diagnostics collections started by tools.
    Collection,
    /// Profiler captures started by tools (a .NET capture): several processes may be recorded at once.
    Capture,
    /// Storage clean-up.
    Maintenance,
}

/// Captures recorded at once (each records another process).
pub const CAPTURES: usize = 4;

impl JobKind {
    #[cfg(test)]
    pub const ALL: [JobKind; 9] = [
        JobKind::Ai,
        JobKind::Research,
        JobKind::Analysis,
        JobKind::Import,
        JobKind::Export,
        JobKind::Download,
        JobKind::Collection,
        JobKind::Capture,
        JobKind::Maintenance,
    ];

    /// How many jobs of this kind run at once (for [`JobKind::Import`]: per window).
    pub fn limit(self, cores: usize) -> usize {
        match self {
            JobKind::Ai | JobKind::Research | JobKind::Import | JobKind::Collection | JobKind::Maintenance => 1,
            JobKind::Analysis => cores.saturating_sub(1).max(1),
            JobKind::Export | JobKind::Download => 2,
            JobKind::Capture => CAPTURES,
        }
    }

    /// Whether the limit counts per window instead of app-wide.
    pub fn per_window(self) -> bool {
        self == JobKind::Import
    }

    /// Whether queued jobs of this kind can be moved up and down.
    pub fn reorderable(self) -> bool {
        self == JobKind::Ai
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Ai => "ai",
            JobKind::Research => "research",
            JobKind::Analysis => "analysis",
            JobKind::Import => "import",
            JobKind::Export => "export",
            JobKind::Download => "download",
            JobKind::Collection => "collection",
            JobKind::Capture => "capture",
            JobKind::Maintenance => "maintenance",
        }
    }
}

/// Where a job is in its life. Finished states never change again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    /// Debrief closed or crashed while the job was queued or running.
    Interrupted,
}

/// What happens to a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobEvent {
    Start,
    Succeed,
    Fail,
    Cancel,
    Interrupt,
}

/// A state change the state machine does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: JobState,
    pub event: JobEvent,
}

impl std::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a {:?} job cannot {:?}", self.from, self.event)
    }
}

impl JobState {
    pub fn is_finished(self) -> bool {
        !matches!(self, JobState::Queued | JobState::Running)
    }

    /// The state after `event`. Queued jobs start, are cancelled, fail (they could not start) or are
    /// interrupted; running jobs end in any finished state; finished jobs stay as they are.
    pub fn step(self, event: JobEvent) -> Result<JobState, IllegalTransition> {
        use JobEvent::*;
        use JobState::*;
        match (self, event) {
            (Queued, Start) => Ok(Running),
            (Running, Succeed) => Ok(Done),
            (Queued | Running, Fail) => Ok(Failed),
            (Queued | Running, Cancel) => Ok(Cancelled),
            (Queued | Running, Interrupt) => Ok(Interrupted),
            (from, event) => Err(IllegalTransition { from, event }),
        }
    }
}

/// What started a job, so the pane can link back to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum JobLink {
    /// An event of the window's logs.
    Event {
        #[serde(rename = "recordId")]
        record_id: u32,
    },
    /// A time range (epoch ms).
    Range { from: i64, to: i64 },
    /// A query (its text, as the user wrote it).
    Query { ql: String },
    /// A file or folder (shown, never read by the UI).
    Path { path: String },
    /// A chat of the window.
    Chat {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    /// A model or runtime of the model manager.
    Model { id: String },
}

/// The window a job belongs to (none for app-wide work) and what started it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSource {
    pub window: Option<String>,
    pub link: Option<JobLink>,
}

/// What `done` and `total` of a [`JobProgress`] count; the UI formats them accordingly (a size, or a plain number).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgressUnit {
    /// Files, pages, events, steps.
    #[default]
    Items,
    /// Bytes transferred or processed.
    Bytes,
}

/// How a phase is measured. Maps 1:1 onto the shared `paddock-jobs` phase scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PhaseScale {
    /// `done`/`total` are bytes.
    Bytes,
    /// `done`/`total` are items.
    #[default]
    Count,
    /// The total is not known (yet): the bar is indeterminate and `done` is a growing count.
    Indeterminate,
}

/// The stage a job is in ("Downloading", "Unpacking"): its own label and scale, shown next to the bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPhase {
    pub label: String,
    #[serde(default)]
    pub scale: PhaseScale,
}

impl JobPhase {
    pub fn new(label: impl Into<String>, scale: PhaseScale) -> JobPhase {
        JobPhase {
            label: label.into(),
            scale,
        }
    }
}

/// How far a job is. Any part may be unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    /// 0..=1.
    pub fraction: Option<f32>,
    pub done: Option<u64>,
    pub total: Option<u64>,
    /// Time left as the producer estimates it (else estimated from the fraction).
    pub eta_ms: Option<u64>,
    /// What `done` and `total` count (absent in older payloads = items).
    #[serde(default)]
    pub unit: ProgressUnit,
    /// Bytes read so far and in all (imports).
    #[serde(default)]
    pub bytes_done: Option<u64>,
    #[serde(default)]
    pub bytes_total: Option<u64>,
    /// What it works on now (an import: the file's name; never a path).
    #[serde(default)]
    pub current: Option<String>,
    /// The stage the job is in now, on its own scale (absent for jobs that have no stages).
    #[serde(default)]
    pub phase: Option<JobPhase>,
}

impl JobProgress {
    /// A named phase on its own scale. `total` 0 or an indeterminate scale gives no fraction (growing `done` only).
    pub fn in_phase(label: &str, scale: PhaseScale, done: u64, total: u64) -> JobProgress {
        let known = total > 0 && scale != PhaseScale::Indeterminate;
        JobProgress {
            fraction: known.then(|| (done as f64 / total as f64).min(1.0) as f32),
            done: Some(done),
            total: known.then_some(total),
            unit: if scale == PhaseScale::Bytes {
                ProgressUnit::Bytes
            } else {
                ProgressUnit::Items
            },
            phase: Some(JobPhase::new(label, scale)),
            ..JobProgress::default()
        }
    }

    pub fn fraction(f: f32) -> JobProgress {
        JobProgress {
            fraction: Some(f),
            ..JobProgress::default()
        }
    }

    pub fn counts(done: u64, total: u64) -> JobProgress {
        JobProgress {
            fraction: (total > 0).then(|| (done as f64 / total as f64) as f32),
            done: Some(done),
            total: (total > 0).then_some(total),
            ..JobProgress::default()
        }
    }

    /// Progress in bytes (a download or copy): the UI shows sizes, speed and time left.
    pub fn bytes(done: u64, total: u64) -> JobProgress {
        JobProgress {
            unit: ProgressUnit::Bytes,
            ..JobProgress::counts(done, total)
        }
    }

    /// Clamps the fraction to 0..=1 and drops non-finite values and counts past the total.
    pub fn sanitized(self) -> JobProgress {
        JobProgress {
            fraction: self.fraction.filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 1.0)),
            ..self
        }
    }
}

/// Where a finished job's result is, for View.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum JobResult {
    /// An AI answer with its citations (explain, summarise, root causes); kept with the job.
    Narrative { narrative: Value },
    /// AI proposals to review (profile, pipeline, markers, extractors, query, regex, folder set-up).
    Proposal { proposal: Value },
    /// The answer of a chat turn, in the window's chat.
    Chat {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    /// A written file (export): View reveals it.
    File { path: String },
    /// The window's workspace (import).
    Workspace,
    /// An analysis view, with the query it ran.
    View { view: String, ql: Option<String> },
    /// A model ready to activate (model manager).
    Model {
        #[serde(rename = "fileName")]
        file_name: Option<String>,
    },
    /// The folder survey's proposals: View reopens the set-up wizard with them.
    FolderSurvey { outcome: Value },
    /// What clearing Debrief's storage freed and kept: View opens Settings ▸ Storage with it.
    Storage { result: Value },
    /// The logs related to an image (ADR 0025): View opens the image with its links.
    RelatedLogs { result: Value },
}

impl JobResult {
    /// Results kept with a job are bounded: an oversized payload is not saved (View then reports it gone).
    pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

    pub fn bounded(self) -> Option<JobResult> {
        let size = match &self {
            JobResult::Narrative { narrative: v }
            | JobResult::Proposal { proposal: v }
            | JobResult::FolderSurvey { outcome: v }
            | JobResult::Storage { result: v }
            | JobResult::RelatedLogs { result: v } => v.to_string().len(),
            _ => 0,
        };
        (size <= Self::MAX_PAYLOAD_BYTES).then_some(self)
    }
}

/// What a producer submits.
#[derive(Debug, Clone, PartialEq)]
pub struct JobSpec {
    pub kind: JobKind,
    pub title: String,
    pub source: JobSource,
    pub cancellable: bool,
    /// What Retry runs again (parsed by the producer's retry handler); `None`: not retryable.
    pub request: Option<Value>,
}

impl JobSpec {
    pub fn new(kind: JobKind, title: impl Into<String>) -> JobSpec {
        JobSpec {
            kind,
            title: title.into(),
            source: JobSource::default(),
            cancellable: true,
            request: None,
        }
    }

    pub fn window(mut self, window: &str) -> JobSpec {
        self.source.window = Some(window.to_string());
        self
    }

    pub fn link(mut self, link: JobLink) -> JobSpec {
        self.source.link = Some(link);
        self
    }

    pub fn not_cancellable(mut self) -> JobSpec {
        self.cancellable = false;
        self
    }

    pub fn retry_with(mut self, request: Value) -> JobSpec {
        self.request = Some(request);
        self
    }
}

/// A job as the queue keeps it and projects save it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub title: String,
    pub source: JobSource,
    /// Changed only by the queue, through [`JobState::step`].
    pub(super) state: JobState,
    #[serde(default)]
    pub progress: JobProgress,
    /// What the job is doing now, or why it ended.
    pub message: Option<String>,
    /// Epoch milliseconds.
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub cancellable: bool,
    pub result: Option<JobResult>,
    /// Why the job failed or was interrupted.
    pub error: Option<String>,
    /// The user looked at the result (View) or it never needed attention.
    #[serde(default)]
    pub seen: bool,
    #[serde(default)]
    pub request: Option<Value>,
    /// The job this one retries.
    #[serde(default)]
    pub retry_of: Option<JobId>,
}

impl Job {
    pub fn state(&self) -> JobState {
        self.state
    }

    /// How long the job ran (to `now` while running); 0 before it started.
    pub fn run_ms(&self, now: i64) -> i64 {
        match self.started_at {
            Some(s) => (self.finished_at.unwrap_or(now) - s).max(0),
            None => 0,
        }
    }

    /// Time since it was submitted (to its end when finished).
    pub fn age_ms(&self, now: i64) -> i64 {
        (self.finished_at.unwrap_or(now) - self.queued_at).max(0)
    }

    pub fn window(&self) -> Option<&str> {
        self.source.window.as_deref()
    }

    /// A finished job as other modules' tests need it (Retry handlers).
    #[cfg(test)]
    pub(crate) fn finished_for_test(spec: JobSpec, state: JobState) -> Job {
        Job {
            id: JobId(1),
            kind: spec.kind,
            title: spec.title,
            source: spec.source,
            state,
            progress: JobProgress::default(),
            message: None,
            queued_at: 0,
            started_at: None,
            finished_at: Some(0),
            cancellable: spec.cancellable,
            result: None,
            error: None,
            seen: false,
            request: spec.request,
            retry_of: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_machine_allows_only_forward_moves() {
        use JobEvent::*;
        use JobState::*;
        assert_eq!(Queued.step(Start), Ok(Running));
        assert_eq!(Running.step(Succeed), Ok(Done));
        assert_eq!(Queued.step(Cancel), Ok(Cancelled));
        assert_eq!(Running.step(Interrupt), Ok(Interrupted));
        assert_eq!(Queued.step(Fail), Ok(Failed));
        assert!(Queued.step(Succeed).is_err(), "a job that never ran cannot succeed");
        assert!(Running.step(Start).is_err());
        for done in [Done, Failed, Cancelled, Interrupted] {
            assert!(done.is_finished());
            for e in [Start, Succeed, Fail, Cancel, Interrupt] {
                let err = done.step(e).unwrap_err();
                assert_eq!(err.from, done);
                assert!(err.to_string().contains("cannot"));
            }
        }
        assert!(!Queued.is_finished() && !Running.is_finished());
    }

    #[test]
    fn limits_follow_the_kind() {
        assert_eq!(JobKind::Ai.limit(16), 1);
        assert_eq!(JobKind::Analysis.limit(8), 7);
        assert_eq!(JobKind::Analysis.limit(1), 1, "at least one");
        assert_eq!(JobKind::Analysis.limit(0), 1);
        assert_eq!(JobKind::Export.limit(8), 2);
        assert_eq!(JobKind::Download.limit(8), 2);
        for k in [JobKind::Import, JobKind::Collection, JobKind::Maintenance] {
            assert_eq!(k.limit(8), 1);
        }
        assert!(JobKind::Import.per_window() && !JobKind::Ai.per_window());
        assert!(JobKind::Ai.reorderable() && !JobKind::Export.reorderable());
        let names: Vec<_> = JobKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "ai",
                "research",
                "analysis",
                "import",
                "export",
                "download",
                "collection",
                "capture",
                "maintenance"
            ]
        );
        assert_eq!(JobKind::Research.limit(8), 1);
        assert_eq!(JobKind::Capture.limit(1), CAPTURES, "captures do not queue behind each other");
        for k in JobKind::ALL {
            assert_eq!(serde_json::to_value(k).unwrap(), k.as_str());
        }
    }

    #[test]
    fn progress_and_results_are_bounded() {
        let p = JobProgress {
            fraction: Some(f32::NAN),
            ..JobProgress::default()
        }
        .sanitized();
        assert_eq!(p.fraction, None);
        assert_eq!(JobProgress::fraction(3.0).sanitized().fraction, Some(1.0));
        assert_eq!(JobProgress::counts(1, 4).fraction, Some(0.25));
        assert_eq!(JobProgress::counts(1, 0).fraction, None);
        let big = JobResult::Narrative {
            narrative: Value::String("x".repeat(JobResult::MAX_PAYLOAD_BYTES + 1)),
        };
        assert!(big.bounded().is_none());
        assert!(JobResult::Workspace.bounded().is_some());
        let link = serde_json::to_value(JobLink::Event { record_id: 4 }).unwrap();
        assert_eq!(link, serde_json::json!({"type": "event", "recordId": 4}));
        assert_eq!(JobId(7).to_string(), "7");
    }
    #[test]
    fn byte_progress_is_typed_and_old_payloads_mean_items() {
        let p = JobProgress::bytes(512, 2048);
        assert_eq!(
            (p.unit, p.done, p.total, p.fraction),
            (ProgressUnit::Bytes, Some(512), Some(2048), Some(0.25))
        );
        assert_eq!(JobProgress::counts(1, 2).unit, ProgressUnit::Items);
        assert_eq!(p.clone().sanitized().unit, ProgressUnit::Bytes);
        assert_eq!(serde_json::to_value(p).unwrap()["unit"], "bytes");
        let old: JobProgress = serde_json::from_value(serde_json::json!({"fraction": 0.5, "done": 1, "total": 2, "etaMs": null})).unwrap();
        assert_eq!(old.unit, ProgressUnit::Items);
    }

    #[test]
    fn phases_have_their_own_scale_and_mirror_the_wire() {
        let b = JobProgress::in_phase("Downloading", PhaseScale::Bytes, 5, 10);
        assert_eq!((b.fraction, b.unit), (Some(0.5), ProgressUnit::Bytes));
        let i = JobProgress::in_phase("Finding files", PhaseScale::Indeterminate, 42, 100);
        assert_eq!((i.fraction, i.total, i.done), (None, None, Some(42)));
        let c = JobProgress::in_phase("Importing", PhaseScale::Count, 3, 0);
        assert_eq!((c.fraction, c.total), (None, None));
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["phase"], serde_json::json!({"label": "Downloading", "scale": "bytes"}));
        let old: JobProgress =
            serde_json::from_value(serde_json::json!({"fraction": null, "done": null, "total": null, "etaMs": null})).unwrap();
        assert_eq!(old.phase, None);
    }
}
