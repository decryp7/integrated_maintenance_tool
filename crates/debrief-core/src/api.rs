//! Request and response types of the store API. Their serde names are the wire
//! format the UI depends on.

use crate::chart::ChartDef;
use crate::model::Level;
use crate::query::Query;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub id: u32,
    pub path: String,
    pub name: String,
    /// Logical log the file belongs to: rolled, dated and rotated files of one
    /// log share it (`logs/app.log` for `logs/app.log.1`), so the UI can collapse them.
    pub group: String,
    pub importer: String,
    pub records: usize,
    pub skipped: usize,
    /// Events cut because they grew past the importer's event limit (the rest continues in a new
    /// event carrying [`crate::importers::text::CONTINUED_FIELD`]).
    pub split_events: usize,
    /// Events removed by the profile's pipeline.
    pub dropped: usize,
    /// Events in/out of each pipeline step for this file.
    #[serde(skip)]
    pub step_stats: Vec<crate::pipeline::StepStats>,
    pub error: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Clock-skew correction applied to this source's timestamps (ms; 0 = none).
    pub offset_ms: i64,
    /// How the source is stored on disk, which decides how it can be opened outside Debrief.
    pub file_kind: FileKind,
    /// Many events without a readable timestamp (see [`crate::timestamp_issue`]).
    pub timestamp_issue: Option<TimestampIssue>,
    /// Display name of the file this logical source was split from by a `[[route]]` (ADR 0020); `None` for a
    /// file's own source. Logical sources of one file share its `path`.
    #[serde(default)]
    pub routed_from: Option<String>,
    /// Import problems of this source, one entry per kind (e.g. lines that look like events but match no
    /// format).
    #[serde(default)]
    pub problems: Vec<crate::ingest::Problem>,
    /// Fields the source's importer and parser declare as identifying who wrote an event (instance-key
    /// candidates).
    #[serde(skip)]
    pub identity: Vec<String>,
    /// Id of the parser that read the source (ADR 0026).
    #[serde(default)]
    pub parser: Option<String>,
    /// How the source's text was decoded and how sure that is (`None` for binary formats and sources read
    /// before encodings were recorded).
    #[serde(default)]
    pub encoding: Option<crate::ingest::EncodingInfo>,
}

/// How a source is stored on disk: what "Open file", "Open at line" and "Show in folder" can do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    /// A plain text file: event line numbers are lines of the file.
    #[default]
    Text,
    /// A file read by a binary importer (event log, trace...): its line numbers are record numbers.
    Binary,
    /// A gzip file: line numbers refer to the decompressed text.
    Compressed,
    /// A member of a zip archive: there is no file of its own to open.
    ArchiveMember,
}

/// A source whose timestamps could not be read for many of its events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampIssue {
    /// Events (lines that start an event) without a readable timestamp.
    pub unparsed_lines: usize,
    /// All events of the source.
    pub total_lines: usize,
    /// Up to three such lines, each cut to a bounded length.
    pub examples: Vec<String>,
    /// The 1-based line number of each example (same order), for jumping to it.
    pub example_lines: Vec<u32>,
    /// A timestamp format the engine checked against those lines, if one reads them.
    pub suggestion: Option<TimestampSuggestion>,
}

/// A timestamp format proposed for a parser's `[format] timestamp_formats`, verified to parse the sample lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampSuggestion {
    /// strftime format, e.g. `%a %b %d %H:%M:%S %Y`.
    pub format: String,
    /// The timestamp text of the first sample, as the format reads it.
    pub sample: String,
    /// The parser lines (`[format]`) that add the format, merged over the source's parser.
    pub parser_snippet: String,
    /// Sample lines (without a readable timestamp) the format reads.
    pub parsed_samples: usize,
    /// Sample lines the format was checked against.
    pub checked_samples: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnrecognizedFile {
    pub path: String,
    pub name: String,
    pub reason: String,
    /// Its importer crashed (contained): a bug to report, not an unknown format.
    #[serde(default)]
    pub crashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldInfo {
    pub name: String,
    pub count: usize,
    pub distinct: usize,
    pub numeric: bool,
    pub derived: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerKind {
    pub kind: String,
    /// Who defines it: `profile`, `builtin` or a plugin id.
    pub owner: String,
    /// Lifecycle meaning of a plugin kind: start, stop or crash.
    pub role: Option<crate::plugin::markers::MarkerRole>,
    pub label: String,
    pub lifecycle: bool,
    pub color: Option<String>,
    /// One of `profile::MARKER_ICONS` or a custom icon name; none = built-in shape.
    pub icon: Option<String>,
    /// Generated badge text (1–3 chars); takes precedence over `icon`.
    pub badge: Option<String>,
    /// Badge outline: circle, square, diamond or hexagon.
    pub shape: Option<String>,
    /// Meaning of the marker, for the legend and tooltips.
    pub description: Option<String>,
    /// The default rule of a plugin kind (all conditions must match), shown
    /// read-only. Empty for profile and built-in kinds (their rules are in
    /// the profile) and for kinds a plugin only marks programmatically.
    pub when: Vec<crate::condition::ConditionCfg>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub sources: Vec<SourceInfo>,
    pub unrecognized: Vec<UnrecognizedFile>,
    pub total: usize,
    /// Events without a timestamp: in the grid (marked), not on the timeline or in time filters.
    pub untimed: usize,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub levels: BTreeMap<Level, usize>,
    pub fields: Vec<FieldInfo>,
    pub default_group_by: Vec<String>,
    pub marker_kinds: Vec<MarkerKind>,
    pub charts: Vec<ChartDef>,
    pub profile_name: String,
    pub load_ms: u64,
    /// Events removed by the profile's pipeline, across all sources.
    pub dropped: usize,
    /// Custom marker icons from the profile (sanitised), by name.
    pub icons: Vec<crate::icons::CustomIcon>,
    /// Events in/out of each `[[pipeline]]` step, summed over all sources.
    pub pipeline: Vec<PipelineStepInfo>,
    pub plugins: Vec<crate::plugin::PluginInfo>,
    pub exporters: Vec<ExporterInfo>,
    /// The profile's dashboards.
    pub dashboards: Vec<crate::dashboard::DashboardCfg>,
    /// Pages, facets and source context are redacted (the "Redacted view").
    pub redacted_view: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRequest {
    pub record_id: u32,
    #[serde(default = "default_radius")]
    pub before: usize,
    #[serde(default = "default_radius")]
    pub after: usize,
}

fn default_radius() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, serde::Deserialize)]
pub struct SourceLine {
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceContext {
    pub source_name: String,
    /// Line of the event itself.
    pub line: u32,
    pub lines: Vec<SourceLine>,
    /// Why the lines are not read from the file (e.g. it is missing, so the stored events are shown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    #[serde(default)]
    pub query: Query,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRequest {
    pub text: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareRequest {
    pub a: Query,
    pub b: Query,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldStatsRequest {
    #[serde(default)]
    pub query: Query,
    pub field: String,
    #[serde(default = "default_histogram")]
    pub buckets: usize,
}

fn default_histogram() -> usize {
    20
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelateRequest {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceRequest {
    #[serde(default)]
    pub query: Query,
    /// Empty: list traces. Set: the waterfall of that trace.
    #[serde(default)]
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceSummary {
    pub trace_id: String,
    pub spans: usize,
    pub errors: u32,
    pub start: i64,
    pub duration_ms: f64,
    pub root: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestsRequest {
    #[serde(default)]
    pub query: Query,
    /// Field(s) identifying a request, `A|B` for alternatives.
    #[serde(default = "default_request_field")]
    pub id_field: String,
}

fn default_request_field() -> String {
    "RequestId|CorrelationId|TraceId".into()
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MomentRequest {
    /// Structural filters (instances, fields) applied to every section; its
    /// time range is replaced by `from`/`to`.
    #[serde(default)]
    pub query: Query,
    pub from: i64,
    pub to: i64,
}

/// How much of a query's matching records the analyses look at: at most a fixed number (the first in
/// time order), so a huge selection stays responsive. `analysed < total` means the results are partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisCoverage {
    pub analysed: usize,
    pub total: usize,
}

impl AnalysisCoverage {
    /// Coverage of `total` matching records under the limit `max`.
    pub fn of(total: usize, max: usize) -> AnalysisCoverage {
        AnalysisCoverage {
            analysed: total.min(max),
            total,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.analysed >= self.total
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MomentSummary {
    pub from: i64,
    pub to: i64,
    pub total: usize,
    pub levels: BTreeMap<Level, usize>,
    pub instances: Vec<MomentInstance>,
    pub metrics: Vec<MetricWindow>,
    /// Most active users, each with the field it was read from so the UI can filter by `field = value`.
    pub users: Vec<UserCount>,
    pub activity: Vec<ActivityItem>,
    pub anomalies: Vec<crate::analysis::Anomaly>,
}

/// How many events in a moment came from one user, and the field that named them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCount {
    /// The `activity.user` field the value was read from (the first one present on the event).
    pub field: String,
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MomentInstance {
    pub key: String,
    pub events: usize,
    pub warnings: usize,
    pub errors: usize,
    pub patterns: Vec<crate::patterns::Pattern>,
    /// First error or fatal event in the window.
    pub first_error: Option<u32>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricWindow {
    /// Instance key, or machine · process for PerfMon counters.
    pub series: String,
    /// Field name, or the PerfMon counter.
    pub metric: String,
    pub samples: usize,
    pub min: f64,
    pub avg: f64,
    pub max: f64,
    pub last: f64,
    /// Average over the window of the same length just before.
    pub previous_avg: Option<f64>,
    pub change_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub id: u32,
    pub ts: i64,
    pub instance: String,
    pub user: Option<String>,
    pub action: String,
    pub level: Level,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsePreviewRequest {
    /// Candidate parser TOML (may be unsaved); empty: the parser the library chooses for the sample.
    pub toml: String,
    /// Candidate profile TOML (may be unsaved); empty: the active profile.
    #[serde(default)]
    pub profile: String,
    /// Sample content, e.g. the first lines of a file.
    pub sample: String,
    /// File name, for importers that look at it.
    #[serde(default)]
    pub file_name: String,
    #[serde(default = "default_preview_records")]
    pub limit: usize,
}

fn default_preview_records() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPreview {
    pub line: u32,
    pub ts: Option<i64>,
    pub level: Level,
    pub message: String,
    pub exception: Option<String>,
    pub fields: Map<String, Value>,
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsePreview {
    pub importer: Option<String>,
    /// Id of the parser that read the sample (ADR 0026).
    #[serde(default)]
    pub parser: Option<String>,
    pub records: Vec<ParsedPreview>,
    pub total: usize,
    pub skipped: usize,
    pub dropped: usize,
    pub error: Option<String>,
}

/// How urgent a finding of the health check is (shared with import problems, `debrief_plugin_api::problems`).
pub use debrief_plugin_api::problems::Severity;

/// What the UI should do when the user follows a finding.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FindingAction {
    /// Zoom to a time range and select an event.
    Zoom { from: i64, to: i64, record_id: Option<u32> },
    /// Show the events of an exception group.
    Exception { fingerprint: String, record_id: Option<u32> },
    /// Open the format wizard for a file.
    SetupFormat { path: String },
    /// Open a panel (e.g. "plugins", "pipeline").
    OpenPanel { panel: String },
}

/// What a finding of the health check is about (the UI picks an icon and counts crashes by it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    Crash,
    ErrorSpike,
    MemoryGrowth,
    Silence,
    SlowOutlier,
    RateChange,
    Exception,
    UnreadLines,
    NoTimestamps,
    FileNotOpened,
    Dropped,
    PluginQuarantined,
    /// Findings left out because there were too many to list.
    NotShown,
}

/// One line of the automatic health check, in plain English.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub action: Option<FindingAction>,
}

/// Opened paths, removed files and clock offsets, enough to rebuild the workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceState {
    pub roots: Vec<String>,
    pub removed: Vec<String>,
    /// Clock-skew offsets by source path.
    pub offsets: crate::skew::ClockOffsets,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PipelineStepInfo {
    pub index: usize,
    pub step: String,
    pub input: usize,
    pub output: usize,
    /// Records the step failed on and left unchanged (`script` runtime errors and limits).
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExporterInfo {
    pub id: String,
    pub name: String,
    pub extension: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    #[serde(default)]
    pub query: Query,
    pub format: String,
    pub path: String,
    #[serde(default)]
    pub descending: bool,
    /// Redact the events with the profile's `[redact]` rules (a fresh salt per export).
    #[serde(default)]
    pub redact: bool,
    /// Replace an existing file at `path` (confirmed by the user); otherwise it is refused.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub format: String,
    pub path: String,
    pub events: usize,
    /// Matches per redaction rule (empty when not redacted).
    #[serde(default)]
    pub redactions: crate::redact::Counts,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub descending: bool,
    #[serde(default)]
    pub anchor: Option<u32>,
}

fn default_limit() -> usize {
    500
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordView<'a> {
    pub id: u32,
    pub ts: Option<i64>,
    pub level: Level,
    /// Borrowed from the record, or owned when the redacted view changed it.
    pub message: Cow<'a, str>,
    pub template: Option<Cow<'a, str>>,
    pub exception: Option<Cow<'a, str>>,
    pub fields: Cow<'a, Map<String, Value>>,
    pub source: u32,
    pub source_name: &'a str,
    pub line: u32,
    pub group: String,
    pub markers: Vec<&'a str>,
    /// Clock-skew offset applied to `ts` (the original is `ts - skewMs`); absent when none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skew_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<'a> {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<RecordView<'a>>,
    pub anchor_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineRequest {
    #[serde(default)]
    pub query: Query,
    #[serde(default = "default_buckets")]
    pub buckets: usize,
    #[serde(default = "yes")]
    pub always_show_lifecycle: bool,
}

fn default_buckets() -> usize {
    400
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerCluster {
    pub kind: String,
    pub bucket: usize,
    pub ts: i64,
    pub id: u32,
    pub count: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lane {
    pub key: String,
    pub first: i64,
    pub last: i64,
    pub total: usize,
    pub matched: usize,
    pub density: Vec<u32>,
    pub markers: Vec<MarkerCluster>,
    /// Ids of the sources with events in the lane, ascending (at most [`MAX_LANE_SOURCES`]).
    pub sources: Vec<u32>,
}

/// Most source ids a timeline lane lists.
pub const MAX_LANE_SOURCES: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub from: i64,
    pub to: i64,
    pub bucket_ms: f64,
    pub histogram: Vec<[u32; 6]>,
    pub lanes: Vec<Lane>,
    pub truncated_lanes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetRequest {
    #[serde(default)]
    pub query: Query,
    pub field: String,
    #[serde(default = "default_facet_limit")]
    pub limit: usize,
}

fn default_facet_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Facet {
    pub field: String,
    pub values: Vec<FacetValue>,
    pub distinct: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartRequest {
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub chart_id: Option<String>,
    #[serde(default)]
    pub def: Option<ChartDef>,
    #[serde(default = "default_buckets")]
    pub buckets: usize,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ChartSeries {
    pub key: String,
    pub points: Vec<(i64, Option<f64>)>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct LifecycleEvent {
    pub kind: String,
    pub ts: i64,
    pub group: String,
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartData {
    pub def: ChartDef,
    pub from: i64,
    pub to: i64,
    pub bucket_ms: i64,
    pub series: Vec<ChartSeries>,
    pub lifecycle: Vec<LifecycleEvent>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractPreviewRequest {
    pub toml: String,
    pub extractor: String,
    #[serde(default)]
    pub query: Query,
    #[serde(default = "default_preview_limit")]
    pub limit: usize,
}

fn default_preview_limit() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ExtractSpan {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractSample {
    pub record_id: u32,
    pub text: String,
    pub spans: Vec<ExtractSpan>,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Default, serde::Deserialize)]
pub struct ExtractPreview {
    pub candidates: usize,
    pub matches: usize,
    pub failed: usize,
    pub samples: Vec<ExtractSample>,
    pub error: Option<String>,
}
