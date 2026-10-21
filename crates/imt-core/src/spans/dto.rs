//! Wire types of runs: the run details on an annotation, the Runs table
//! request/reply, per-kind summaries and the guided editor's preview.

use super::config::SpanIssue;
use super::pairing::{RunEvent, RunStatus};
use crate::plugin::annotate::{AnnotationDto, FieldMatch};
use serde::{Deserialize, Serialize};

/// Most runs one [`RunsReply`] page holds.
pub const MAX_RUNS_PAGE: usize = 1_000;
/// Most lanes a [`RunsRequest`] may name.
pub const MAX_REQUEST_INSTANCES: usize = 10_000;
/// Runs (and anomalies, separately) a preview lists.
pub const PREVIEW_RUNS: usize = 20;

/// What a run annotation adds to [`AnnotationDto`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    /// The `[[span]]` kind.
    pub span: String,
    pub span_label: String,
    pub status: RunStatus,
    pub start: Option<RunEvent>,
    pub stop: Option<RunEvent>,
    /// `to - from` (0 for a stop without start).
    pub duration_ms: i64,
    /// Longer than the rule's `max_duration`.
    pub too_long: bool,
    /// The field value it was paired by (`per = "field:<Name>"`).
    pub group: Option<FieldMatch>,
    pub color: Option<String>,
}

/// Counts and durations of one span kind's runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanSummary {
    pub kind: String,
    pub label: String,
    pub color: Option<String>,
    pub enabled: bool,
    /// Runs of every status.
    pub runs: usize,
    pub closed: usize,
    pub open: usize,
    pub no_stop: usize,
    pub no_start: usize,
    pub too_long: usize,
    /// Lanes with at least one run.
    pub lanes: usize,
    /// Over runs with a duration (all but stops without start).
    pub total_ms: i64,
    pub avg_ms: i64,
    pub max_ms: i64,
    /// Runs dropped by the per-lane cap, and events ignored by the group cap.
    pub dropped: usize,
    /// Marker kinds the rule names that are not defined (or switched off).
    pub missing_markers: Vec<String>,
}

impl SpanSummary {
    /// Anomalies: runs without a start or stop, and runs over `max_duration`.
    pub fn anomalies(&self) -> usize {
        self.no_stop + self.no_start + self.too_long
    }

    pub(crate) fn add(&mut self, info: &RunInfo, lane_new: bool) {
        self.runs += 1;
        match info.status {
            RunStatus::Closed => self.closed += 1,
            RunStatus::Open => self.open += 1,
            RunStatus::NoStop => self.no_stop += 1,
            RunStatus::NoStart => self.no_start += 1,
        }
        if info.too_long {
            self.too_long += 1;
        }
        if lane_new {
            self.lanes += 1;
        }
        if info.status != RunStatus::NoStart {
            self.total_ms = self.total_ms.saturating_add(info.duration_ms);
            self.max_ms = self.max_ms.max(info.duration_ms);
        }
    }

    pub(crate) fn finish(&mut self) {
        let timed = self.runs - self.no_start;
        self.avg_ms = if timed == 0 { 0 } else { self.total_ms / timed as i64 };
    }
}

/// Column the Runs table sorts by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunSort {
    #[default]
    Start,
    Stop,
    Duration,
    Status,
    Lane,
}

/// Which runs `query_runs` returns; every key is optional.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunsRequest {
    /// Overlapping `[from, to]` (inclusive); open-ended when absent.
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Only these lanes; empty = every lane.
    pub instances: Vec<String>,
    /// Only this span kind.
    pub kind: Option<String>,
    /// Only these statuses; empty = all.
    pub statuses: Vec<RunStatus>,
    pub sort: RunSort,
    pub descending: bool,
    pub offset: usize,
    /// Page size (default 200, at most [`MAX_RUNS_PAGE`]).
    pub limit: Option<usize>,
}

/// A page of runs plus the summaries of every span kind over the selection.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsReply {
    pub summaries: Vec<SpanSummary>,
    /// The profile's `[[span]]` rules as written (switched-off ones too), TOML-shaped.
    pub rules: Vec<super::SpanCfg>,
    /// Run annotations (each with `run`), sorted as requested.
    pub runs: Vec<AnnotationDto>,
    /// Runs matching the request (before paging).
    pub total: usize,
    pub offset: usize,
}

/// What starts or stops a run in the guided editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Trigger {
    /// Existing marker kinds.
    Markers { kinds: Vec<String> },
    /// A regex over the message; saving creates a marker rule for it.
    Pattern {
        pattern: String,
        #[serde(default)]
        case_sensitive: bool,
    },
}

/// A candidate `[[span]]` rule from the guided editor.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanDraft {
    pub kind: String,
    #[serde(default)]
    pub label: String,
    pub start: Trigger,
    pub stop: Trigger,
    /// `"instance"` or `"field:<Name>"`.
    #[serde(default = "instance")]
    pub per: String,
    #[serde(default)]
    pub max_duration: Option<String>,
    #[serde(default)]
    pub restart_on_start: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
}

fn instance() -> String {
    "instance".into()
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanPreviewRequest {
    pub draft: SpanDraft,
}

/// What saving a draft writes into the profile (TOML-shaped, snake_case keys):
/// the whole new `[[span]]` list (the draft replacing a rule of the same kind,
/// else added last; the profile's other rules kept) and the `[[marker]]`
/// rules its text patterns need (each replacing one of the same kind).
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanPlan {
    /// The draft's `[[span]]` table.
    pub span: serde_json::Value,
    /// Every `[[span]]` table of the profile after saving.
    pub spans: Vec<serde_json::Value>,
    pub markers: Vec<serde_json::Value>,
}

/// The guided editor's live preview of a draft over the loaded data.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanPreview {
    /// Problems that prevent saving, by form field.
    pub errors: Vec<SpanIssue>,
    /// Worth knowing, but saving is possible (e.g. replaces an existing rule).
    pub warnings: Vec<String>,
    /// Over the loaded data (absent while there are errors).
    pub summary: Option<SpanSummary>,
    /// The first runs by start time.
    pub runs: Vec<AnnotationDto>,
    /// The first anomalies (no start, no stop, too long) by start time.
    pub anomalies: Vec<AnnotationDto>,
    /// What saving writes (absent while there are errors).
    pub plan: Option<SpanPlan>,
}
