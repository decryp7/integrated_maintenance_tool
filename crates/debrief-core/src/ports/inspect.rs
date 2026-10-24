//! What the Store pane shows (UX audit C1, ADR 0014 amendment in ADR 0030): read-only reports about the store
//! itself (its folder, segments, fields, full-text index and memory) and the maintenance it can run. Everything is
//! computed by the store; the UI only formats it. camelCase on the wire; with the `ts` feature the types derive
//! ts-rs and the app writes them to `app/src/app/core/generated/store.ts`.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Resident memory the store worker may use while importing (ADR 0014 §2).
pub const IMPORT_BUDGET_BYTES: u64 = 768 << 20;
/// Resident memory the store worker may use while idle or answering queries (ADR 0014 §2).
pub const QUERY_BUDGET_BYTES: u64 = 300 << 20;
/// Most segments of one page of [`SegmentsRequest`].
pub const MAX_SEGMENT_ROWS: usize = 500;
/// Most sources listed per segment (the count is exact).
pub const SEGMENT_SOURCES: usize = 8;
/// Most fields [`Inspect::store_fields`](super::Inspect::store_fields) lists (the store tracks at most 512 per
/// segment).
pub const MAX_FIELD_ROWS: usize = 2_000;

/// What the store is doing now.
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "StoreActivity"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoreActivity {
    Ready,
    Importing,
    /// The full-text index is catching up with new segments.
    Indexing,
}

/// The last import of this store (kept in its catalog, so it survives reopening).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastImport {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub events: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub elapsed_ms: u64,
    /// Events per second over the whole import (0 when it took no measurable time).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub events_per_second: u64,
    /// When it ended (Unix ms).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub finished_ms: i64,
}

impl LastImport {
    /// The import's numbers with its speed.
    pub fn new(events: u64, bytes: u64, elapsed_ms: u64, finished_ms: i64) -> LastImport {
        let events_per_second = events.saturating_mul(1000).checked_div(elapsed_ms).unwrap_or(0);
        LastImport {
            events,
            bytes,
            elapsed_ms,
            events_per_second,
            finished_ms,
        }
    }
}

/// The Store pane's overview.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreOverview {
    /// The store folder.
    pub dir: String,
    /// Everything in the folder (database, segments, index, spill files).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub events: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub sources: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub segments: u64,
    /// Bumped by every change to the events (cached analyses belong to one version).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub data_version: u64,
    /// The database layout version.
    pub format_version: u32,
    pub activity: StoreActivity,
    pub last_import: Option<LastImport>,
    /// Segments the next compaction rewrites (rows of removed sources, or too many small segments).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub pending_compaction: u64,
}

/// A page of the segment list, in id order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct SegmentsRequest {
    pub offset: usize,
    /// At most [`MAX_SEGMENT_ROWS`] (0: that many).
    pub limit: usize,
}

impl SegmentsRequest {
    /// The page size, bounded.
    pub fn rows(&self) -> usize {
        match self.limit {
            0 => MAX_SEGMENT_ROWS,
            n => n.min(MAX_SEGMENT_ROWS),
        }
    }
}

/// One Parquet segment.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentRow {
    pub file: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub first_id: u64,
    /// One past the last id it may hold.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end_id: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub rows: u64,
    /// Rows of sources that are still in the store (the rest go at the next compaction).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub live: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size_bytes: u64,
    /// Earliest and latest event time (Unix ms), to the minute.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub from: Option<i64>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub to: Option<i64>,
    /// The Parquet compression codec (e.g. `ZSTD`).
    pub compression: String,
    /// The first [`SEGMENT_SOURCES`] source ids of its rows.
    pub sources: Vec<u32>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub source_count: u64,
    /// In the full-text index.
    pub indexed: bool,
    /// Fields stored in their own column in this segment.
    pub promoted: Vec<String>,
    /// The next compaction rewrites it.
    pub pending_compaction: bool,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentsPage {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: u64,
    pub offset: usize,
    pub rows: Vec<SegmentRow>,
}

/// How a field is stored.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldStorage {
    /// A column the profile declares (typed, in every segment).
    Declared,
    /// Its own column in every segment that has it (promoted when at least half of a segment's first batch has it).
    Promoted,
    /// Its own column in some segments, the overflow map in the others.
    Partly,
    /// The overflow map (`extra`) only: filtering on it reads the map, which is slower.
    Overflow,
}

/// The value type of a field, as the store sees it.
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "StoreFieldType"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    Text,
    Integer,
    Number,
    Boolean,
}

/// One field of the store.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreField {
    pub key: String,
    #[serde(rename = "type")]
    pub ty: FieldType,
    pub storage: FieldStorage,
    /// Segments in which it has its own column, and segments that hold it at all.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub promoted_segments: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub segments: u64,
    /// Events that have it.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub events: u64,
    /// `events` over all events, 0..=1.
    pub fill_rate: f64,
    /// Approximate distinct values: the largest per-segment estimate (a lower bound across segments).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub approx_distinct: u64,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreFields {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub events: u64,
    pub fields: Vec<StoreField>,
    /// More fields than [`MAX_FIELD_ROWS`]: the rarest were left out.
    pub truncated: bool,
}

/// The full-text index (Tantivy).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreIndex {
    /// Documents in the index, and events in the store.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub docs: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub events: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size_bytes: u64,
    /// Tantivy's segments.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub index_segments: u64,
    /// Store segments not indexed yet (search does not see their events).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub pending_segments: u64,
    pub catching_up: bool,
    /// The last commit (Unix ms).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_commit_ms: Option<i64>,
    /// The index writer's heap and threads.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub writer_heap_bytes: u64,
    pub writer_threads: u32,
    /// Why the index is unavailable, or the last indexing failure.
    pub error: Option<String>,
}

/// A job running for the window, as the Store pane lists it.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningJob {
    pub kind: String,
    pub title: String,
}

/// The store worker process, as the worker reports it.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerMemory {
    pub pid: u32,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub rss_bytes: u64,
    /// Started in Low memory mode.
    pub low_memory: bool,
}

/// Memory and work of the store against its budgets.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreMemory {
    /// The worker process (absent when the store runs in the app's process, as in tests).
    pub worker: Option<WorkerMemory>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub import_budget_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub query_budget_bytes: u64,
    /// DuckDB's memory limit (as configured, e.g. `192MB`) and in bytes.
    pub duck_limit: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub duck_limit_bytes: u64,
    /// What DuckDB's buffer manager holds now, and what it spilled to the store's `tmp` folder.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub duck_used_bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub duck_spilled_bytes: u64,
    pub duck_threads: u32,
    /// The SQL console's own memory limit (inside the query budget).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub console_limit_bytes: u64,
    pub importing: bool,
    /// Jobs of the window now (filled by the app).
    pub running_jobs: Vec<RunningJob>,
    /// Analyses kept for the current data version, and their size.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub cached_analyses: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub cached_bytes: u64,
}

/// Maintenance the Store pane offers. Each runs as a confirmed job and is refused (busy) while an import runs.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaintenanceAction {
    /// Merge small segments and drop rows of removed sources now.
    Compact,
    /// Throw the full-text index away and index every segment again.
    RebuildIndex,
    /// Forget the cached analyses (they are computed again on next use).
    ClearCachedAnalyses,
}

impl MaintenanceAction {
    /// The phase the job reports first.
    pub fn phase(self) -> &'static str {
        match self {
            MaintenanceAction::Compact => "Compacting segments",
            MaintenanceAction::RebuildIndex => "Clearing the index",
            MaintenanceAction::ClearCachedAnalyses => "Clearing cached analyses",
        }
    }

    /// The job's title.
    pub fn title(self) -> &'static str {
        match self {
            MaintenanceAction::Compact => "Compact the store",
            MaintenanceAction::RebuildIndex => "Rebuild the full-text index",
            MaintenanceAction::ClearCachedAnalyses => "Clear cached analyses",
        }
    }
}

/// What a maintenance action did, in one line.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReport {
    pub action: MaintenanceAction,
    pub message: String,
}

/// A DuckDB size setting (`192MB`, `1.5GiB`, `128 MiB`) in bytes: `KB`/`MB`/`GB`/`TB` are powers of 1000, `KiB`/`MiB`/
/// `GiB`/`TiB` of 1024, as DuckDB reads them. `None` when it is not one.
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" | "bytes" => 1.0,
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = n * mult;
    (bytes < u64::MAX as f64).then_some(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_maintenance_action_has_a_phase_and_a_title() {
        let all = [
            (MaintenanceAction::Compact, "Compacting segments", "Compact the store"),
            (MaintenanceAction::RebuildIndex, "Clearing the index", "Rebuild the full-text index"),
            (
                MaintenanceAction::ClearCachedAnalyses,
                "Clearing cached analyses",
                "Clear cached analyses",
            ),
        ];
        for (a, phase, title) in all {
            assert_eq!((a.phase(), a.title()), (phase, title));
        }
    }

    #[test]
    fn sizes_read_like_duckdb() {
        assert_eq!(parse_size("192MB"), Some(192_000_000));
        assert_eq!(parse_size("128 MiB"), Some(128 << 20));
        assert_eq!(parse_size("1.5GiB"), Some(3 << 29));
        assert_eq!(parse_size("2kb"), Some(2000));
        assert_eq!(parse_size("7"), Some(7));
        assert_eq!(parse_size("1TB"), Some(1_000_000_000_000));
        assert_eq!(parse_size("1 TiB"), Some(1 << 40));
        assert_eq!(parse_size("12 parsecs"), None);
        assert_eq!(parse_size("MB"), None);
        assert_eq!(parse_size("-1MB"), None);
        assert_eq!(parse_size("99999999999999999999TiB"), None);
    }

    #[test]
    fn speeds_pages_and_titles() {
        let l = LastImport::new(10_000, 5, 2_000, 1);
        assert_eq!(l.events_per_second, 5_000);
        assert_eq!(LastImport::new(10, 0, 0, 0).events_per_second, 0);
        assert_eq!(SegmentsRequest::default().rows(), MAX_SEGMENT_ROWS);
        assert_eq!(SegmentsRequest { offset: 0, limit: 9_999 }.rows(), MAX_SEGMENT_ROWS);
        assert_eq!(SegmentsRequest { offset: 0, limit: 3 }.rows(), 3);
        let json = serde_json::to_value(MaintenanceAction::ClearCachedAnalyses).unwrap();
        assert_eq!(json, "clearCachedAnalyses");
        for a in [
            MaintenanceAction::Compact,
            MaintenanceAction::RebuildIndex,
            MaintenanceAction::ClearCachedAnalyses,
        ] {
            assert!(!a.title().is_empty());
        }
        let f = StoreField {
            key: "k".into(),
            ty: FieldType::Integer,
            storage: FieldStorage::Partly,
            promoted_segments: 1,
            segments: 2,
            events: 3,
            fill_rate: 0.5,
            approx_distinct: 4,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!((v["type"].as_str(), v["storage"].as_str()), (Some("integer"), Some("partly")));
        assert_eq!(v["fillRate"], 0.5);
    }
}
