//! The read-only SQL console (UX audit C2, ADR 0030): requests, results and its limits. The store runs the SQL in
//! a sandboxed DuckDB of its own over documented, versioned views (`imt.events`, `imt.sources`, `imt.fields`,
//! `imt.notes`); see the adapter for the sandbox and the checks. camelCase on the wire; ts-rs with `ts`.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Most rows a run keeps (the rest are counted as cut off).
pub const SQL_MAX_ROWS: usize = 10_000;
/// Rows per page.
pub const SQL_PAGE_ROWS: usize = 500;
/// A run is stopped after this long.
pub const SQL_TIME_LIMIT: Duration = Duration::from_secs(30);
/// The console's DuckDB memory limit (inside the worker's 300 MB query budget).
pub const SQL_MEMORY_LIMIT: &str = "128MB";
/// The console's DuckDB threads.
pub const SQL_THREADS: usize = 2;
/// Longest SQL text.
pub const MAX_SQL_LEN: usize = 64 << 10;
/// Longest cell text; longer values are cut and end with `…`.
pub const MAX_CELL_CHARS: usize = 4_096;
/// Most bytes of cell text a run keeps (a result of long values stops earlier than [`SQL_MAX_ROWS`]).
pub const SQL_MAX_RESULT_BYTES: usize = 32 << 20;
/// Most notes a run sees, and the longest note text it sees.
pub const SQL_MAX_NOTES: usize = 5_000;
pub const SQL_NOTE_CHARS: usize = 2_048;
/// The version of the views' columns (documented in ADR 0030 and docs/user-guide.md). Bumped only when a column
/// changes meaning or goes away; added columns keep it.
pub const SQL_VIEWS_VERSION: u32 = 1;

/// A note as the `imt.notes` view sees it (the app owns notes and hands them over with each run).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SqlNote {
    pub id: String,
    /// The anchor's kind (`event`, `range`, `instant`, `source`, `marker`, `chartPoint`, `project`, `image`).
    pub anchor: String,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub event_id: Option<u64>,
    /// The noted source's path, for source notes (`imt.notes.source_id` is the source with that path).
    pub source_path: Option<String>,
    /// Unix ms.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub from: Option<i64>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub to: Option<i64>,
    pub severity: String,
    pub tags: Vec<String>,
    pub text: String,
    pub author: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub updated: i64,
}

/// One run of the console.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SqlRequest {
    pub sql: String,
    /// The query plan (`EXPLAIN`) instead of rows.
    pub explain: bool,
    /// What `imt.notes` holds (the app fills it from the window's notes).
    pub notes: Vec<SqlNote>,
}

/// A result column: its name and DuckDB type.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

/// What a run returned: the columns, the first page and how to page further.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlResult {
    /// The result's handle for [`SqlPageRequest`] (the store keeps the last few).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub result: u64,
    pub columns: Vec<SqlColumn>,
    /// The first [`SQL_PAGE_ROWS`] rows; cells are JSON scalars (null, boolean, number, string).
    #[cfg_attr(feature = "ts", ts(type = "Array<Array<string | number | boolean | null>>"))]
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Rows kept (at most [`SQL_MAX_ROWS`]).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: u64,
    /// The query returned more rows than were kept.
    pub truncated: bool,
    /// The rows carry event ids: a column `id` of an integer type (from `imt.events`).
    pub event_ids: bool,
    /// The query plan, when [`SqlRequest::explain`] asked for it (then there are no rows).
    pub plan: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub elapsed_ms: u64,
}

/// More rows of a result.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlPageRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub result: u64,
    pub offset: usize,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlPage {
    pub offset: usize,
    #[cfg_attr(feature = "ts", ts(type = "Array<Array<string | number | boolean | null>>"))]
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// A documented column of a view.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlViewColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub description: String,
}

/// One of the console's views.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlView {
    /// Its name in the `imt` schema (also reachable unqualified).
    pub name: String,
    pub description: String,
    pub columns: Vec<SqlViewColumn>,
}

/// What the console's editor completes: the views and their columns, and the functions it may call.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlSchema {
    pub version: u32,
    pub views: Vec<SqlView>,
    /// Scalar and aggregate functions (names), and the table functions the console allows.
    pub functions: Vec<String>,
    pub table_functions: Vec<String>,
    /// The limits, for the editor's hints.
    pub max_rows: usize,
    pub page_rows: usize,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub time_limit_ms: u64,
}

/// Most rows a CSV export of a console query writes (the rest are counted as cut off).
pub const SQL_EXPORT_MAX_ROWS: u64 = 5_000_000;
/// Most bytes a CSV export writes; it stops before the row that would cross it.
pub const SQL_EXPORT_MAX_BYTES: u64 = 2 << 30;
/// A CSV export is stopped after this long (it runs as a job; Cancel stops it earlier).
pub const SQL_EXPORT_TIME_LIMIT: Duration = Duration::from_secs(600);

/// The event ids of a kept result (its `id` column), for *Open in Events* on the whole result.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlIdsRequest {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub result: u64,
}

/// The ids of a result's rows, in row order without repeats (at most [`SQL_MAX_ROWS`]; NULL ids left out).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlIds {
    #[cfg_attr(feature = "ts", ts(type = "number[]"))]
    pub ids: Vec<u64>,
    /// The query returned more rows than the result kept: these are the ids of the kept rows only.
    pub truncated: bool,
}

/// Runs a console query again and writes all its rows (up to [`SQL_EXPORT_MAX_ROWS`]) as CSV to `path`.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SqlExportRequest {
    pub sql: String,
    /// What `imt.notes` holds (as for a run).
    pub notes: Vec<SqlNote>,
    /// The file the user chose.
    pub path: String,
    /// Replace an existing file at `path` (confirmed by the user); otherwise it is refused.
    pub overwrite: bool,
}

/// What a CSV export wrote.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlExportResult {
    pub path: String,
    /// Data rows written (the header row not counted).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub rows: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// The query had more rows than the export's limits let it write.
    pub truncated: bool,
}
