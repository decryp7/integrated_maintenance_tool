//! Running a playbook: one step against the open data ([`run_step`]) and the recorded results of a run, which the
//! project keeps ([`PlaybookRecord`], versioned, tolerant of damage).

use super::{Action, Expect, Playbook, Step};
use crate::known_issues::evidence;
use crate::ports::{Store, StoreResult};
use crate::ql::QlResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Runs kept in a project (the oldest are dropped).
pub const MAX_RUNS: usize = 50;
/// Rows of a query result a step returns.
const MAX_ROWS: usize = 20;
const MAX_NOTE: usize = 4_000;
const RECORD_VERSION: u32 = 1;

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    /// A check met its expectation, or a manual step was done.
    Pass,
    /// A check did not meet its expectation.
    Fail,
    /// Information: a query result, a filter, a view.
    Info,
    /// The step could not run (bad query, store failure); the summary says what to fix.
    Error,
    /// The user skipped it.
    Skipped,
}

/// What running a step found. The engine computes it; the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub status: StepStatus,
    /// One sentence ("Found 12 events", "Expected no events, found 3").
    pub summary: String,
    /// Events matched by a query, filter or check.
    pub count: Option<usize>,
    pub sample_ids: Vec<u32>,
    /// A table result (query steps with `stats`), at most 20 rows.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl StepResult {
    fn plain(status: StepStatus, summary: impl Into<String>) -> StepResult {
        StepResult {
            status,
            summary: summary.into(),
            count: None,
            sample_ids: Vec::new(),
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }
}

fn plural(n: usize) -> String {
    format!("{n} event{}", if n == 1 { "" } else { "s" })
}

/// Runs one step. Manual and view steps compute nothing; queries, filters and checks run in the store. Queries never
/// change anything. A query that does not work (syntax, unknown field) is a result with status `Error`, not a failure
/// of the call; only store failures are returned as errors.
pub fn run_step(store: &dyn Store, step: &Step) -> StoreResult<StepResult> {
    Ok(match &step.action {
        Action::Manual => StepResult::plain(StepStatus::Info, "Do this by hand, then mark it done."),
        Action::OpenView { view } => StepResult::plain(StepStatus::Info, format!("Open the {view} view.")),
        Action::Filter { ql } => match evidence(store, ql)? {
            Err(e) => StepResult::plain(StepStatus::Error, format!("The filter does not work here: {e}")),
            Ok((n, ids)) => StepResult {
                count: Some(n),
                sample_ids: ids,
                ..StepResult::plain(StepStatus::Info, format!("The filter matches {}.", plural(n)))
            },
        },
        Action::Query { ql } => run_query_step(store, ql)?,
        Action::Check { ql, expect } => match evidence(store, ql)? {
            Err(e) => StepResult::plain(StepStatus::Error, format!("The check does not work here: {e}")),
            Ok((n, ids)) => {
                let (ok, wanted) = match *expect {
                    Expect::NoMatch => (n == 0, "no events".to_string()),
                    Expect::AnyMatch => (n > 0, "at least one event".to_string()),
                    Expect::AtLeast { n: m } => (n >= m, format!("at least {}", plural(m))),
                    Expect::AtMost { n: m } => (n <= m, format!("at most {}", plural(m))),
                };
                StepResult {
                    count: Some(n),
                    sample_ids: ids,
                    ..StepResult::plain(
                        if ok { StepStatus::Pass } else { StepStatus::Fail },
                        format!("Expected {wanted}, found {}.", n),
                    )
                }
            }
        },
    })
}

fn run_query_step(store: &dyn Store, ql: &str) -> StoreResult<StepResult> {
    let req = crate::api::QueryRequest {
        query: crate::query::Query {
            ql: ql.to_string(),
            ..Default::default()
        },
    };
    Ok(match store.run_query(&req) {
        Ok(QlResult::Events { ids, .. }) => StepResult {
            count: Some(ids.len()),
            sample_ids: ids.iter().copied().take(crate::known_issues::MAX_SAMPLES).collect(),
            ..StepResult::plain(StepStatus::Info, format!("Found {}.", plural(ids.len())))
        },
        Ok(QlResult::Table { columns, mut rows }) => {
            let n = rows.len();
            rows.truncate(MAX_ROWS);
            StepResult {
                count: Some(n),
                columns,
                rows,
                ..StepResult::plain(StepStatus::Info, format!("The query returned {n} row{}.", if n == 1 { "" } else { "s" }))
            }
        }
        Err(crate::ports::StoreError::Query(e)) => StepResult::plain(StepStatus::Error, format!("The query does not work here: {}", e.message)),
        Err(e) => return Err(e),
    })
}

/// What was recorded for one step of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRecord {
    pub step_id: String,
    pub status: StepStatus,
    pub summary: String,
    pub count: Option<usize>,
    pub sample_ids: Vec<u32>,
    /// The user's note on this step.
    #[serde(default)]
    pub note: String,
    /// When (epoch ms).
    pub at: i64,
}

/// One execution of a playbook, with the playbook as it was (a later edit of the library copy does not rewrite
/// history).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookRun {
    pub id: String,
    pub playbook: Playbook,
    pub started_at: i64,
    #[serde(default)]
    pub steps: Vec<StepRecord>,
}

impl PlaybookRun {
    /// A new run. `id` must be non-empty.
    pub fn start(id: &str, playbook: Playbook, now: i64) -> anyhow::Result<PlaybookRun> {
        playbook.check()?;
        if id.trim().is_empty() {
            anyhow::bail!("a run needs an id");
        }
        Ok(PlaybookRun {
            id: id.to_string(),
            playbook,
            started_at: now,
            steps: Vec::new(),
        })
    }

    /// Records (or replaces) the result of step `step_id`. The step must be in the run's playbook.
    pub fn record(&mut self, step_id: &str, result: &StepResult, note: &str, now: i64) -> anyhow::Result<()> {
        if !self.playbook.steps.iter().any(|s| s.id == step_id) {
            anyhow::bail!("the playbook has no step \"{}\"", step_id.chars().take(40).collect::<String>());
        }
        if note.chars().count() > MAX_NOTE {
            anyhow::bail!("a note is at most {MAX_NOTE} characters");
        }
        let rec = StepRecord {
            step_id: step_id.to_string(),
            status: result.status,
            summary: result.summary.clone(),
            count: result.count,
            sample_ids: result.sample_ids.clone(),
            note: note.to_string(),
            at: now,
        };
        match self.steps.iter_mut().find(|r| r.step_id == step_id) {
            Some(r) => *r = rec,
            None => self.steps.push(rec),
        }
        Ok(())
    }

    /// How far the run is: (steps recorded, steps in the playbook).
    pub fn progress(&self) -> (usize, usize) {
        (self.steps.len(), self.playbook.steps.len())
    }
}

/// The runs a project keeps (versioned, like the notes and the knowledge record).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookRecord {
    pub version: u32,
    pub runs: Vec<PlaybookRun>,
}

impl PlaybookRecord {
    /// An empty record of the current version.
    pub fn new() -> PlaybookRecord {
        PlaybookRecord {
            version: RECORD_VERSION,
            runs: Vec::new(),
        }
    }

    /// Reads a saved record. A newer or damaged record restores nothing and says why; damaged runs inside a good
    /// record are skipped (each named in the warnings).
    pub fn restore(value: Value) -> (PlaybookRecord, Vec<String>) {
        let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
        if version > u64::from(RECORD_VERSION) {
            return (PlaybookRecord::new(), vec![format!("the saved playbook runs are from a newer version of the app (format {version}) and were not restored")]);
        }
        let Some(runs) = value.get("runs").and_then(Value::as_array) else {
            return (PlaybookRecord::new(), vec!["the saved playbook runs are damaged and were not restored".into()]);
        };
        let mut out = PlaybookRecord::new();
        let mut warnings = Vec::new();
        for (i, r) in runs.iter().enumerate().take(MAX_RUNS) {
            match serde_json::from_value::<PlaybookRun>(r.clone()).map_err(|e| e.to_string()).and_then(|run| {
                run.playbook.check().map(|()| run).map_err(|e| e.to_string())
            }) {
                Ok(run) => out.runs.push(run),
                Err(e) => warnings.push(format!("playbook run {} was skipped: {e}", i + 1)),
            }
        }
        if runs.len() > MAX_RUNS {
            warnings.push(format!("only the first {MAX_RUNS} playbook runs were restored"));
        }
        (out, warnings)
    }

    /// Adds a run (replacing one with the same id); the oldest runs beyond [`MAX_RUNS`] are dropped.
    pub fn put(&mut self, run: PlaybookRun) {
        match self.runs.iter_mut().find(|r| r.id == run.id) {
            Some(r) => *r = run,
            None => self.runs.push(run),
        }
        if self.runs.len() > MAX_RUNS {
            let extra = self.runs.len() - MAX_RUNS;
            self.runs.drain(..extra);
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.runs.len();
        self.runs.retain(|r| r.id != id);
        self.runs.len() != before
    }
}
