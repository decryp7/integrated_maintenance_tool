//! Request and response types of live tail. Their serde names are the wire
//! format the UI depends on (`app/src/app/core/api.types.ts`).

use crate::analysis::AnomalyKind;
use crate::api::Severity;
use crate::query::Query;
use serde::{Deserialize, Serialize};

/// What happens to the records of a file that disappears while tailed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeletedFiles {
    /// Keep the records already imported (history is never lost silently).
    #[default]
    Keep,
    /// Drop the file's source and records.
    Drop,
}

/// Engine-side live tail settings (the watcher settings live with the shell).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TailOptions {
    pub deleted_files: DeletedFiles,
    /// An unterminated last line is parsed anyway after this long without a newline.
    pub carry_flush_ms: u64,
    /// The live anomaly detectors run at most this often (rules and silences every batch).
    pub detect_every_ms: u64,
}

/// Longest `detect_every_ms` accepted (10 minutes); longer values are clamped.
pub const MAX_DETECT_EVERY_MS: u64 = 600_000;

impl TailOptions {
    /// The options with every value in its accepted range (`detect_every_ms`
    /// at most [`MAX_DETECT_EVERY_MS`]; 0 runs the detectors after every batch).
    pub fn clamped(mut self) -> TailOptions {
        self.detect_every_ms = self.detect_every_ms.min(MAX_DETECT_EVERY_MS);
        self
    }
}

impl Default for TailOptions {
    fn default() -> Self {
        TailOptions {
            deleted_files: DeletedFiles::Keep,
            carry_flush_ms: 2_000,
            detect_every_ms: 1_000,
        }
    }
}

/// Options of an open request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OpenOptions {
    /// Tail only lines written from now on, skipping the files' history.
    pub start_at_end: bool,
}

/// A source whose file was renamed (rotation) and keeps its records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatedSource {
    pub source_id: u32,
    pub from: String,
    pub to: String,
}

/// What one live refresh changed; the payload of `imt://workspace-changed`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailReport {
    /// New records (not counting records rewritten in place).
    pub appended: usize,
    /// Existing records rewritten in place (continuation lines merged into them).
    pub updated: usize,
    /// Sources whose records changed in any way.
    pub sources_changed: Vec<u32>,
    /// Sources created for files that appeared.
    pub added: Vec<u32>,
    pub rotated: Vec<RotatedSource>,
    /// Sources whose file was truncated or rewritten (copy-truncate); the file
    /// is read again from its start as a new source.
    pub truncated: Vec<u32>,
    /// Sources whose file disappeared (records kept unless `deletedFiles: drop`).
    pub deleted: Vec<u32>,
    /// Sources read again in full (binary formats, archives, inconsistencies).
    pub reimported: Vec<u32>,
    /// Record ids at and after this one changed (records were inserted before
    /// newer ones, or removed); `None` when every existing id is stable.
    pub ids_changed_from: Option<u32>,
    /// Timestamp of the newest record.
    pub newest_ts: Option<i64>,
    /// Instance keys seen for the first time.
    pub instances_changed: Vec<String>,
    /// Files whose live state changed in this refresh (new files included;
    /// files dropped with `deletedFiles: drop` are listed in `deleted` only).
    pub file_states: Vec<TailFileChange>,
    /// Number of `imt://live-alert` alerts raised by this refresh.
    pub alerts_raised: usize,
    /// The alerts themselves (emitted one by one as `imt://live-alert`).
    #[serde(skip)]
    pub alerts: Vec<LiveAlert>,
    pub elapsed_ms: f64,
}

impl TailReport {
    /// Whether anything the UI shows changed.
    pub fn is_empty(&self) -> bool {
        self.appended == 0
            && self.updated == 0
            && self.sources_changed.is_empty()
            && self.added.is_empty()
            && self.rotated.is_empty()
            && self.truncated.is_empty()
            && self.deleted.is_empty()
            && self.reimported.is_empty()
            && self.file_states.is_empty()
            && self.alerts.is_empty()
    }

    /// Number of sources (re)imported, appended to or added.
    pub fn changed_sources(&self) -> usize {
        self.sources_changed.len()
    }
}

/// Live state of one tailed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TailFileState {
    Tailing,
    /// Renamed by rotation; still read until the writer lets go.
    Rotated,
    /// Truncated or rewritten in place; its records stay, the file restarted as a new source.
    Truncated,
    /// Gone from disk; its records stay (or were dropped).
    Deleted,
    /// Read only as a whole (binary format or archive).
    Whole,
    Error {
        message: String,
    },
}

/// A file whose live state changed in a refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailFileChange {
    pub source_id: u32,
    pub state: TailFileState,
}

/// How changes to a file are noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatchMode {
    /// Operating-system change notifications.
    Events,
    /// Periodic polling (network shares, or after a watcher error).
    Polling,
    /// Not watched (live tail off).
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailFileStatus {
    pub source_id: u32,
    pub path: String,
    pub name: String,
    pub group: String,
    pub mode: WatchMode,
    /// Bytes turned into records.
    pub offset: u64,
    /// File size at the last read.
    pub size: u64,
    pub lines_per_sec: f64,
    /// Wall-clock time (ms) of the last change seen.
    pub last_event: Option<i64>,
    pub state: TailFileState,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailTotals {
    pub files: usize,
    pub tailing: usize,
    pub errors: usize,
    pub bytes: u64,
    pub lines_per_sec: f64,
    pub records: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailStatus {
    pub files: Vec<TailFileStatus>,
    pub totals: TailTotals,
}

pub use crate::query::TimeWindow;

/// A finding of the live detectors or a user alert rule; the payload of `imt://live-alert`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveAlert {
    /// Stable id: the same finding gets the same id, so the UI can drop
    /// duplicates. It is `kind:who:bucket`, where `who` is the rule name (for
    /// `rule`) or the instance (empty when none) and `bucket` is `from`
    /// divided by the profile's `[live] cooldown` (rounded down): within one
    /// cooldown the same kind and instance (or rule) is raised at most once.
    pub id: String,
    /// An anomaly kind (`errorSpike`, `silence`, `crash`, `slowOutlier`,
    /// `rateChange`, `memoryGrowth`) or `rule`.
    pub kind: String,
    /// The alert rule's name for `rule`.
    pub rule: Option<String>,
    pub instance: Option<String>,
    pub severity: Severity,
    pub from: i64,
    pub to: i64,
    pub message: String,
    /// The query that shows the evidence (time range, instance, levels).
    pub evidence_query: Query,
}

/// What a mute applies to: one anomaly kind (`{ "kind": "errorSpike" }`) or
/// one alert rule by name (`{ "rule": "payment errors" }`). Exactly one of
/// the two, so a mute can never name both or neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AlertTarget {
    Kind(AnomalyKind),
    Rule(String),
}
