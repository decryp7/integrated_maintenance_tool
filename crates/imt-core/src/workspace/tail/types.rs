//! Request and response types of live tail. Their serde names are the wire
//! format the UI depends on (`app/src/app/core/api.types.ts`).

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

/// Bounded memory for long live sessions. Both limits are off by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Retention {
    /// Keep at most this many records (the newest).
    pub max_records: Option<usize>,
    /// Keep only records at most this old, relative to the newest record.
    pub max_age_ms: Option<i64>,
}

/// Engine-side live tail settings (the watcher settings live with the shell).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TailOptions {
    pub deleted_files: DeletedFiles,
    pub retention: Retention,
    /// An unterminated last line is parsed anyway after this long without a newline.
    pub carry_flush_ms: u64,
    /// The live anomaly detectors run at most this often (rules and silences every batch).
    pub detect_every_ms: u64,
}

impl Default for TailOptions {
    fn default() -> Self {
        TailOptions {
            deleted_files: DeletedFiles::Keep,
            retention: Retention::default(),
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
    /// Records removed by the retention limit.
    pub evicted: usize,
    /// Record ids at and after this one changed (records were inserted before
    /// newer ones, removed or evicted); `None` when every existing id is stable.
    pub ids_changed_from: Option<u32>,
    /// Timestamp of the newest record.
    pub newest_ts: Option<i64>,
    /// Instance keys seen for the first time.
    pub instances_changed: Vec<String>,
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
            && self.evicted == 0
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
    /// Records evicted by retention since the workspace was opened.
    pub evicted: usize,
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
