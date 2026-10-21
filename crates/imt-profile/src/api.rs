//! Request and response DTOs of the profiling engine (the wire contract with
//! the UI, mirrored in `app/src/app/core/api.types.ts`, camelCase JSON).
//! Requests are parsed here once (serde) and validated by the operations.

use crate::error::{ErrorCode, ProfileError};
use crate::model::{EventKind, FrameKind, Measurement, MetricId, MetricInfo, ProfileKind, Thread, Unit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Values per metric, e.g. `{ "samples": 12, "timeNs": 12000000 }` (only the profile's metrics).
pub type MetricValues = BTreeMap<MetricId, u64>;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The error every profiling command rejects with: a code the UI acts on and
/// a message for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilingErrorDto {
    pub code: ErrorCode,
    pub message: String,
}

impl ProfilingErrorDto {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> ProfilingErrorDto {
        ProfilingErrorDto {
            code,
            message: message.into(),
        }
    }
}

impl From<ProfileError> for ProfilingErrorDto {
    fn from(e: ProfileError) -> Self {
        ProfilingErrorDto::new(e.code(), e.to_string())
    }
}

impl std::fmt::Display for ProfilingErrorDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// ---------------------------------------------------------------------------
// Captures
// ---------------------------------------------------------------------------

/// What a file looks like to the importers, without importing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SniffKind {
    /// A CPU / allocation / wall-clock profile.
    Capture,
    /// A heap snapshot.
    Heap,
    /// Not a capture (e.g. a log file).
    None,
}

/// [`SniffKind`] of one path (`profiling_sniff`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SniffResult {
    pub path: String,
    pub kind: SniffKind,
}

/// A metric of a profile, with how it was measured (label estimates in the UI).
pub type MetricDto = MetricInfo;

/// An open capture: a profile or a heap snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "capture", rename_all = "camelCase")]
pub enum CaptureSummary {
    Profile(ProfileSummary),
    Heap(HeapSummary),
}

impl CaptureSummary {
    pub fn id(&self) -> u32 {
        match self {
            CaptureSummary::Profile(p) => p.id,
            CaptureSummary::Heap(h) => h.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: u32,
    pub name: String,
    /// The file it was opened from.
    pub path: Option<String>,
    /// Importer id (`speedscope`, `pprof`, `nettrace`, ...).
    pub format: String,
    pub kind: ProfileKind,
    /// Wall-clock start (Unix ms) when the format records it; lines the capture up with logs.
    pub started_ms: Option<i64>,
    pub duration_ns: u64,
    /// Samples have timestamps: the thread timeline and time ranges work.
    pub has_time: bool,
    pub sample_count: u64,
    pub frame_count: u64,
    pub event_count: u64,
    pub metrics: Vec<MetricDto>,
    pub default_metric: MetricId,
    pub threads: Vec<Thread>,
    /// Event kinds present on the timeline.
    pub event_kinds: Vec<EventKind>,
    /// Import notes (truncations, unresolved addresses, skipped records).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapSummary {
    pub id: u32,
    pub name: String,
    pub path: Option<String>,
    pub format: String,
    pub taken_ms: Option<i64>,
    pub type_count: u64,
    pub object_count: u64,
    pub total_bytes: u64,
    /// The object graph is present: retained sizes, dominators and paths to roots work.
    pub has_graph: bool,
    pub notes: Vec<String>,
}

/// A capture's time span on the log timeline (only captures with a wall-clock start).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSpan {
    pub id: u32,
    pub name: String,
    pub capture: CaptureKind,
    pub kind: Option<ProfileKind>,
    pub from_ms: i64,
    pub to_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureKind {
    Profile,
    Heap,
}

// ---------------------------------------------------------------------------
// Views over a profile
// ---------------------------------------------------------------------------

/// `[fromNs, toNs]` (inclusive) in nanoseconds since the profile's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub from_ns: u64,
    pub to_ns: u64,
}

/// How recursion is shown in trees and flame graphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecursionMode {
    /// As recorded.
    #[default]
    Off,
    /// `a → a → a` becomes `a`.
    Direct,
    /// A frame already on the path folds back into that ancestor (`a → b → a → c` becomes `a → b`, `a → c`).
    All,
}

/// "Just my code": runs of framework/system frames collapse into the first
/// (outermost) of them, which keeps their time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JustMyCode {
    pub enabled: bool,
    /// Extra patterns (`*` wildcard, case-insensitive) matched against a frame's module and function.
    pub framework_patterns: Vec<String>,
    /// Also use the importer's framework flag on each frame (built-in rules).
    pub use_frame_flags: bool,
}

impl Default for JustMyCode {
    fn default() -> Self {
        JustMyCode {
            enabled: true,
            framework_patterns: Vec::new(),
            use_frame_flags: true,
        }
    }
}

/// Filters and transforms shared by every view of one profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileView {
    /// Only samples in this time range (ignored for profiles without timestamps).
    pub range: Option<TimeRange>,
    /// Only these threads (ids from the summary); all when absent.
    pub threads: Option<Vec<u32>>,
    pub just_my_code: Option<JustMyCode>,
    pub recursion: RecursionMode,
}

/// A frame as rows and rectangles reference it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameDto {
    pub id: u32,
    pub name: String,
    pub module: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub kind: FrameKind,
    pub is_framework: bool,
}

/// Which value of a node to sort or measure by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueOf {
    #[serde(rename = "self")]
    SelfValue,
    #[default]
    Total,
    /// The metric's total divided by the calls (needs exact call counts); rows without calls sort last.
    AvgPerCall,
}

/// Sort order of a table: by a metric (self or total) or by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SortSpec {
    /// The metric to sort by; `null` sorts by name.
    pub metric: Option<MetricId>,
    pub of: ValueOf,
    /// Default true for metrics; names sort ascending unless set.
    pub descending: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupBy {
    #[default]
    Function,
    Type,
    Namespace,
    Module,
    Thread,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotspotsRequest {
    pub profile_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default)]
    pub group_by: GroupBy,
    /// Metric for percentages (default: the profile's default metric).
    #[serde(default)]
    pub metric: Option<MetricId>,
    #[serde(default)]
    pub sort: Option<SortSpec>,
    /// Case-insensitive text matched against the name and module.
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

pub fn default_limit() -> usize {
    200
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotspotRow {
    /// Stable key of the group (pass it back as `function` to the butterfly view for function rows).
    pub key: String,
    pub name: String,
    pub module: Option<String>,
    /// A representative frame (function grouping only).
    pub frame: Option<FrameDto>,
    /// The thread (thread grouping only).
    pub thread: Option<u32>,
    #[serde(rename = "self")]
    pub self_values: MetricValues,
    #[serde(rename = "total")]
    pub total_values: MetricValues,
    pub self_pct: f64,
    pub total_pct: f64,
    /// Total time / calls, when the profile has exact call counts.
    pub avg_per_call_ns: Option<f64>,
    /// Longest single call, when recorded.
    pub max_per_call_ns: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotspotsPage {
    pub rows: Vec<HotspotRow>,
    /// Rows matching the search (before paging).
    pub total_rows: usize,
    pub grand_total: MetricValues,
    pub metric: MetricId,
    pub measurement: Measurement,
    /// Samples in the view (after range and thread filters).
    pub samples_in_view: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TreeDirection {
    /// Callers above callees (call tree).
    #[default]
    TopDown,
    /// Leaf functions first, their callers below (inverted tree).
    BottomUp,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeRequest {
    pub profile_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default)]
    pub direction: TreeDirection,
    /// Frame ids from the top of the tree to the node whose children are wanted; empty = the top level.
    #[serde(default)]
    pub path: Vec<u32>,
    #[serde(default)]
    pub metric: Option<MetricId>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub frame: FrameDto,
    #[serde(rename = "self")]
    pub self_values: MetricValues,
    #[serde(rename = "total")]
    pub total_values: MetricValues,
    pub self_pct: f64,
    pub total_pct: f64,
    pub has_children: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreePage {
    /// Children sorted by the metric's total, descending.
    pub nodes: Vec<TreeNode>,
    pub total_children: usize,
    pub grand_total: MetricValues,
    pub metric: MetricId,
    pub measurement: Measurement,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotPathRequest {
    pub profile_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default)]
    pub direction: TreeDirection,
    /// Where to start (empty = the top).
    #[serde(default)]
    pub path: Vec<u32>,
    #[serde(default)]
    pub metric: Option<MetricId>,
    /// Stop when the hottest child is below this share of the grand total (default 0.01).
    #[serde(default)]
    pub min_fraction: Option<f64>,
}

/// The hottest top-down call paths that reach a function (`profiling_frame_paths`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramePathsRequest {
    pub profile_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    /// A frame id; every frame of the same function (name and module) counts.
    pub frame: u32,
    #[serde(default)]
    pub metric: Option<MetricId>,
    /// Paths to return (default 5, at most 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One top-down path from a root to an occurrence of the function.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FramePath {
    /// Frame ids from the root; pass as a call-tree path or a flame graph focus.
    pub path: Vec<u32>,
    /// The frames of `path`, in order (for labels).
    pub frames: Vec<FrameDto>,
    /// Inclusive value of the occurrence (the metric of the reply).
    pub value: u64,
    /// `value` as a share of the view's total (percent).
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FramePaths {
    /// Hottest first. Nested (recursive) occurrences are not listed separately.
    pub paths: Vec<FramePath>,
    /// Occurrences in the view before the limit; 0 when the function is not in this view.
    pub total_paths: usize,
    pub metric: MetricId,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotPath {
    /// Frame ids of the whole path from the top (the request's path, then the expansion).
    pub path: Vec<u32>,
    /// The expanded nodes, in order.
    pub nodes: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ButterflyRequest {
    pub profile_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    /// A frame id; every frame of the same function (name and module) is included.
    pub frame: u32,
    #[serde(default)]
    pub metric: Option<MetricId>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ButterflyRow {
    /// `null` for the top of the stack (the function was a root).
    pub frame: Option<FrameDto>,
    pub values: MetricValues,
    /// Share of the function's total (metric of the request).
    pub pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Butterfly {
    pub function: HotspotRow,
    /// Who called it, by inclusive value through that caller, descending.
    pub callers: Vec<ButterflyRow>,
    /// What it called, by inclusive value, descending.
    pub callees: Vec<ButterflyRow>,
    pub metric: MetricId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlameOrder {
    /// Alphabetical siblings (classic flame graph; merges stay stable while zooming).
    #[default]
    Name,
    /// Heaviest sibling first.
    Value,
}

/// Visible part of the focused node's width, both in `[0, 1]`, `start < end`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub start: f64,
    pub end: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { start: 0.0, end: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameRequest {
    pub profile_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    /// TopDown: flame graph / icicle of the call tree; BottomUp: of the inverted tree.
    #[serde(default)]
    pub direction: TreeDirection,
    /// Path of the node zoomed into (empty = everything).
    #[serde(default)]
    pub focus: Vec<u32>,
    #[serde(default)]
    pub viewport: Viewport,
    /// Width of the drawing area in CSS pixels.
    pub width_px: f64,
    /// Rectangles narrower than this are left out (default 0.5 px).
    #[serde(default)]
    pub min_width_px: Option<f64>,
    #[serde(default)]
    pub max_depth: Option<u32>,
    #[serde(default)]
    pub metric: Option<MetricId>,
    #[serde(default)]
    pub order: FlameOrder,
    /// Highlights frames whose name, module or file contains this (case-insensitive), or matches it as a regex.
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub search_regex: bool,
    /// Cap on rectangles (default 50,000).
    #[serde(default)]
    pub max_rects: Option<usize>,
}

/// One rectangle: `x`/`w` are fractions of the drawing width (viewport applied), `depth` 0 is the focus level.
/// Flame graph = depth grows upwards; icicle = depth grows downwards (the UI decides).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameRect {
    pub frame: u32,
    /// Index of the enclosing rectangle in `rects`; `null` at depth 0.
    pub parent: Option<u32>,
    pub depth: u32,
    pub x: f64,
    pub w: f64,
    pub value: u64,
    pub self_value: u64,
    /// Matches the search.
    pub matched: bool,
    /// Differential graphs only: target share minus baseline share of the grand total (−1..1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<f64>,
    /// Differential graphs only: the baseline's value scaled to the target's total.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameGraph {
    pub rects: Vec<FlameRect>,
    /// Every frame the rectangles reference.
    pub frames: Vec<FrameDto>,
    /// The frames of the request's `focus` path, in order (zoom breadcrumbs).
    pub focus_frames: Vec<FrameDto>,
    /// Value of the focused node (the full width before the viewport).
    pub focus_value: u64,
    pub grand_total: u64,
    /// Sum of matched values without counting nested matches twice (whole focus, not just visible rects).
    pub matched_value: u64,
    pub matched_pct: f64,
    /// More rectangles were visible than `maxRects`.
    pub truncated: bool,
    pub max_depth: u32,
    pub metric: MetricId,
    pub measurement: Measurement,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFlameRequest {
    pub baseline_id: u32,
    pub target_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default)]
    pub focus: Vec<u32>,
    #[serde(default)]
    pub viewport: Viewport,
    pub width_px: f64,
    #[serde(default)]
    pub min_width_px: Option<f64>,
    #[serde(default)]
    pub max_depth: Option<u32>,
    #[serde(default)]
    pub metric: Option<MetricId>,
    #[serde(default)]
    pub order: FlameOrder,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub search_regex: bool,
    #[serde(default)]
    pub max_rects: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDiffRequest {
    pub baseline_id: u32,
    pub target_id: u32,
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default)]
    pub group_by: GroupBy,
    #[serde(default)]
    pub metric: Option<MetricId>,
    #[serde(default)]
    pub sort: DiffSort,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffSort {
    /// Largest change of self share first (either direction).
    #[default]
    AbsDeltaSelf,
    AbsDeltaTotal,
    /// Biggest growth of self share first (regressions).
    DeltaSelf,
    DeltaTotal,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffStatus {
    /// Only in the target.
    New,
    /// Only in the baseline.
    Removed,
    Changed,
    Same,
}

/// One side of a comparison: raw values and shares of that profile's grand total (percent).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSide {
    #[serde(rename = "self")]
    pub self_value: u64,
    #[serde(rename = "total")]
    pub total_value: u64,
    pub self_pct: f64,
    pub total_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDiffRow {
    pub key: String,
    pub name: String,
    pub module: Option<String>,
    /// A frame of the target profile (for navigation), when the function is in it.
    pub target_frame: Option<u32>,
    pub baseline: DiffSide,
    pub target: DiffSide,
    /// Percentage points (target share − baseline share).
    pub delta_self_pct: f64,
    pub delta_total_pct: f64,
    pub status: DiffStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDiffPage {
    pub rows: Vec<FunctionDiffRow>,
    pub total_rows: usize,
    pub baseline_total: u64,
    pub target_total: u64,
    pub metric: MetricId,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTimelineRequest {
    pub profile_id: u32,
    /// Range and threads apply; frame transforms do not matter here.
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default = "default_buckets")]
    pub buckets: usize,
    #[serde(default)]
    pub metric: Option<MetricId>,
    /// Event kinds to include (all when absent).
    #[serde(default)]
    pub event_kinds: Option<Vec<EventKind>>,
    #[serde(default)]
    pub max_events: Option<usize>,
}

pub fn default_buckets() -> usize {
    400
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLane {
    pub thread: Thread,
    pub total: u64,
    /// One value per bucket (the metric's sum; maximum for `maxCallNs`).
    pub buckets: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDto {
    pub kind: EventKind,
    pub thread: Option<u32>,
    pub start_ns: u64,
    pub dur_ns: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTimeline {
    pub from_ns: u64,
    pub to_ns: u64,
    pub bucket_ns: f64,
    /// Threads with activity in the range, busiest first.
    pub threads: Vec<ThreadLane>,
    pub events: Vec<EventDto>,
    pub events_truncated: bool,
    pub metric: MetricId,
    /// Wall-clock start of the profile (Unix ms), to label the axis.
    pub started_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

/// Thresholds (percent of the view's total unless noted); every key optional.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FindingThresholds {
    /// Report functions whose self share is at least this.
    pub top_self_pct: f64,
    /// At most this many top-self findings.
    pub top_self_count: usize,
    pub gc_pct: f64,
    pub lock_pct: f64,
    /// Exceptions: minimum count (exact data) or share of samples in throw machinery.
    pub exception_count: u64,
    pub exception_pct: f64,
    /// Percentage points a function's share must grow to be a regression.
    pub regression_pct: f64,
    /// Share of pool-thread samples that are blocking waits.
    pub starvation_blocked_pct: f64,
}

impl Default for FindingThresholds {
    fn default() -> Self {
        FindingThresholds {
            top_self_pct: 5.0,
            top_self_count: 5,
            gc_pct: 10.0,
            lock_pct: 5.0,
            exception_count: 1,
            exception_pct: 1.0,
            regression_pct: 5.0,
            starvation_blocked_pct: 50.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsRequest {
    pub profile_id: u32,
    /// Compare against this profile for regressions.
    #[serde(default)]
    pub baseline_id: Option<u32>,
    #[serde(default)]
    pub view: ProfileView,
    #[serde(default)]
    pub thresholds: FindingThresholds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    TopSelf,
    GcPressure,
    LockContention,
    ExceptionHotPath,
    ThreadPoolStarvation,
    Regression,
    /// One call path carries most of the time (the call tree opens along it).
    HotPath,
    /// Threads spent much of their time blocked or waiting for a CPU (context-switch waits).
    Blocking,
    /// Just-in-time compilation took a noticeable share of the time.
    JitTime,
    /// What allocates the most (types and call sites, large object heap).
    Allocations,
    /// The user-code native function with the most self time, with its source location (ADR 0022).
    HotNativeFunction,
    /// The native function in which threads waited longest (locks, blocking calls).
    WaitHotSpot,
    /// The call path holding most native heap memory not freed by the end of the capture.
    NativeHeapGrowth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingSeverity {
    Info,
    Warning,
    Critical,
}

/// Which view shows a finding's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceView {
    Hotspots,
    CallTree,
    Flame,
    Butterfly,
    Timeline,
    Diff,
}

/// Where to look: open `view` on `profileId` with this selection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub view: EvidenceView,
    pub profile_id: u32,
    pub baseline_id: Option<u32>,
    pub frame: Option<FrameDto>,
    /// Call-tree path (frame ids, top-down) to expand.
    pub path: Vec<u32>,
    pub metric: MetricId,
    pub range: Option<TimeRange>,
    pub threads: Option<Vec<u32>>,
    pub event_kind: Option<EventKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileFinding {
    pub kind: FindingKind,
    pub severity: FindingSeverity,
    pub title: String,
    pub detail: String,
    /// The finding's headline number (share in percent), when it has one.
    pub value_pct: Option<f64>,
    pub evidence: Evidence,
}

// ---------------------------------------------------------------------------
// Log timeline integration
// ---------------------------------------------------------------------------

/// "Profile samples in this window" for the Moment view (wall-clock Unix ms).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMomentRequest {
    pub from_ms: i64,
    pub to_ms: i64,
    /// Top functions per profile (default 5).
    #[serde(default)]
    pub top: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMoment {
    pub profile_id: u32,
    pub name: String,
    /// The window as a range of the profile (pass it as `view.range`).
    pub range: TimeRange,
    pub samples: u64,
    pub metric: MetricId,
    pub measurement: Measurement,
    pub value: u64,
    /// Hottest functions by self value in the window.
    pub top: Vec<HotspotRow>,
}

// ---------------------------------------------------------------------------
// Heap snapshots
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeapSort {
    Name,
    Count,
    #[default]
    Bytes,
    Retained,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapTypesRequest {
    pub snapshot_id: u32,
    #[serde(default)]
    pub sort: HeapSort,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapTypeRow {
    pub name: String,
    pub count: u64,
    pub bytes: u64,
    /// Bytes freed if every object of the type went away (needs the object graph).
    pub retained: Option<u64>,
    pub bytes_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapTypesPage {
    pub rows: Vec<HeapTypeRow>,
    pub total_rows: usize,
    pub total_bytes: u64,
    pub total_count: u64,
    pub has_graph: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapDiffRequest {
    pub baseline_id: u32,
    pub target_id: u32,
    #[serde(default)]
    pub search: Option<String>,
    /// Default: bytes growth, largest first.
    #[serde(default)]
    pub sort: HeapDiffSort,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeapDiffSort {
    #[default]
    BytesDelta,
    CountDelta,
    Name,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapDiffRow {
    pub name: String,
    pub count_before: u64,
    pub count_after: u64,
    pub count_delta: i64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub bytes_delta: i64,
    /// Not in the baseline.
    pub is_new: bool,
    /// Not in the target.
    pub is_gone: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapDiffPage {
    pub rows: Vec<HeapDiffRow>,
    pub total_rows: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub count_before: u64,
    pub count_after: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapGrowthRequest {
    /// Three or more snapshots, oldest first.
    pub snapshot_ids: Vec<u32>,
    #[serde(default)]
    pub search: Option<String>,
    /// Only types that grew in every step.
    #[serde(default)]
    pub steady_only: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapGrowthRow {
    pub name: String,
    /// Bytes per snapshot, in request order.
    pub bytes: Vec<u64>,
    pub counts: Vec<u64>,
    /// Least-squares growth in bytes per snapshot.
    pub slope_bytes: f64,
    /// Last minus first.
    pub growth_bytes: i64,
    /// Never shrank and grew overall (bytes or count): a leak candidate.
    pub steady: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapGrowthPage {
    pub rows: Vec<HeapGrowthRow>,
    pub total_rows: usize,
    pub steady_count: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapDominatorsRequest {
    pub snapshot_id: u32,
    /// The object whose dominated children are wanted; `null` = the top (children of the roots).
    #[serde(default)]
    pub parent: Option<u32>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapObjectRow {
    pub object: u32,
    pub type_name: String,
    pub shallow: u64,
    pub retained: u64,
    /// Objects it immediately dominates.
    pub child_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapObjectsPage {
    /// Sorted by retained size, descending.
    pub rows: Vec<HeapObjectRow>,
    pub total_rows: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapInstancesRequest {
    pub snapshot_id: u32,
    pub type_name: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapPathsRequest {
    pub snapshot_id: u32,
    pub object: u32,
    /// Default 5, at most 20.
    #[serde(default)]
    pub max_paths: Option<usize>,
    /// Default and cap 256.
    #[serde(default)]
    pub max_depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathStep {
    pub object: u32,
    pub type_name: String,
    /// The reference from the previous step to this object (field name, `[index]`); `null` on the first step.
    pub edge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapPaths {
    /// Each path runs from an object a GC root references to the requested object; shortest first.
    pub paths: Vec<Vec<PathStep>>,
    /// The object is not reachable from any root (garbage at snapshot time).
    pub unreachable: bool,
    /// The search stopped at its visit limit; more paths may exist.
    pub truncated: bool,
}

/// `profiling_heap_findings`: findings for a snapshot, with growth against `baselineId`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapFindingsRequest {
    pub snapshot_id: u32,
    #[serde(default)]
    pub baseline_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HeapFindingKind {
    /// The type that keeps the most memory alive.
    TopRetainers,
    /// Objects of 85,000 bytes or more (the .NET large object heap).
    LargeObjects,
    /// Objects waiting for their finalizer.
    Finalizable,
    /// Static fields retaining much of the heap.
    StaticRetention,
    /// Delegates and event handlers retaining their targets.
    EventHandlers,
    /// A string-heavy heap.
    Strings,
    /// Types that grew since the baseline snapshot.
    Growth,
}

/// The memory view that shows a heap finding's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HeapFindingView {
    Types,
    Dominators,
    Paths,
    Diff,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapFinding {
    pub kind: HeapFindingKind,
    pub severity: FindingSeverity,
    pub title: String,
    pub detail: String,
    pub value_bytes: Option<u64>,
    /// The type to select in `view`, when the finding is about one.
    pub type_name: Option<String>,
    pub view: HeapFindingView,
}

/// Units for formatting values in the UI (re-exported for completeness).
pub type MetricUnit = Unit;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_with_defaults() {
        let r: HotspotsRequest = serde_json::from_str(r#"{"profileId":1}"#).unwrap();
        assert_eq!(r.limit, 200);
        assert_eq!(r.group_by, GroupBy::Function);
        assert_eq!(r.view, ProfileView::default());
        let r: TreeRequest = serde_json::from_str(
            r#"{"profileId":1,"direction":"bottomUp","path":[1,2],"view":{"range":{"fromNs":1,"toNs":2},"threads":[0],"justMyCode":{"enabled":true},"recursion":"all"}}"#,
        )
        .unwrap();
        assert_eq!(r.direction, TreeDirection::BottomUp);
        let jmc = r.view.just_my_code.unwrap();
        assert!(jmc.use_frame_flags, "defaults fill missing keys");
        assert_eq!(r.view.recursion, RecursionMode::All);
        let s: SortSpec = serde_json::from_str(r#"{"metric":"timeNs","of":"self"}"#).unwrap();
        assert_eq!(s.of, ValueOf::SelfValue);
        let f: FlameRequest = serde_json::from_str(r#"{"profileId":1,"widthPx":800}"#).unwrap();
        assert_eq!(f.viewport, Viewport::default());
        let t: FindingsRequest = serde_json::from_str(r#"{"profileId":1,"thresholds":{"gcPct":3}}"#).unwrap();
        assert_eq!(t.thresholds.gc_pct, 3.0);
        assert_eq!(t.thresholds.lock_pct, 5.0);
        let tl: ThreadTimelineRequest = serde_json::from_str(r#"{"profileId":1}"#).unwrap();
        assert_eq!(tl.buckets, 400);
        assert!(serde_json::from_str::<HotspotsRequest>(r#"{"profileId":1,"groupBy":"nope"}"#).is_err());
    }

    #[test]
    fn summaries_serialise_tagged_and_camel_case() {
        let h = CaptureSummary::Heap(HeapSummary {
            id: 3,
            name: "h".into(),
            path: None,
            format: "heapsnapshot".into(),
            taken_ms: None,
            type_count: 1,
            object_count: 2,
            total_bytes: 3,
            has_graph: true,
            notes: vec![],
        });
        assert_eq!(h.id(), 3);
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["capture"], "heap");
        assert_eq!(v["hasGraph"], true);
        let mut values = MetricValues::new();
        values.insert(MetricId::TimeNs, 5);
        assert_eq!(serde_json::to_value(&values).unwrap()["timeNs"], 5);
    }

    #[test]
    fn errors_sniffs_and_new_keys_serialise_for_the_ui() {
        let e: ProfilingErrorDto = ProfileError::Unrecognised("a.log".into(), "x".into()).into();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "notACapture");
        assert!(v["message"].as_str().unwrap().contains("a.log"));
        assert_eq!(e.to_string(), e.message);
        let io = ProfilingErrorDto::new(ErrorCode::Io, "cannot read");
        assert_eq!(
            serde_json::to_value(&io).unwrap(),
            serde_json::json!({ "code": "io", "message": "cannot read" })
        );
        let sniff = SniffResult {
            path: "/a".into(),
            kind: SniffKind::None,
        };
        assert_eq!(serde_json::to_value(&sniff).unwrap()["kind"], "none");
        assert_eq!(serde_json::to_value(SniffKind::Capture).unwrap(), "capture");
        assert_eq!(serde_json::to_value(SniffKind::Heap).unwrap(), "heap");
        let s: SortSpec = serde_json::from_str(r#"{"metric":"timeNs","of":"avgPerCall"}"#).unwrap();
        assert_eq!(s.of, ValueOf::AvgPerCall);
        let r: FramePathsRequest = serde_json::from_str(r#"{"profileId":1,"frame":4}"#).unwrap();
        assert_eq!((r.frame, r.limit, r.metric), (4, None, None));
        let rect = FlameRect {
            frame: 1,
            parent: None,
            depth: 0,
            x: 0.0,
            w: 1.0,
            value: 1,
            self_value: 0,
            matched: false,
            delta: None,
            baseline_value: None,
        };
        let v = serde_json::to_value(&rect).unwrap();
        assert!(v["parent"].is_null(), "parent is always present: {v}");
        assert!(v.get("delta").is_none());
    }
}
