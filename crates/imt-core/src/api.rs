//! Request and response types of the store API. Their serde names are the wire
//! format the UI depends on.

use crate::chart::ChartDef;
use crate::model::Level;
use crate::query::Query;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub id: u32,
    pub path: String,
    pub name: String,
    pub importer: String,
    pub records: usize,
    pub skipped: usize,
    /// Events removed by the profile's pipeline.
    pub dropped: usize,
    /// Events in/out of each pipeline step for this file.
    #[serde(skip)]
    pub step_stats: Vec<crate::pipeline::StepStats>,
    pub error: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnrecognizedFile {
    pub path: String,
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
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
    pub label: String,
    pub lifecycle: bool,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub sources: Vec<SourceInfo>,
    pub unrecognized: Vec<UnrecognizedFile>,
    pub total: usize,
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
    /// Events in/out of each `[[pipeline]]` step, summed over all sources.
    pub pipeline: Vec<PipelineStepInfo>,
    pub plugins: Vec<crate::plugin::PluginInfo>,
    pub exporters: Vec<ExporterInfo>,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceLine {
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceContext {
    pub source_name: String,
    /// Line of the event itself.
    pub line: u32,
    pub lines: Vec<SourceLine>,
}

/// Opened paths and removed files, enough to rebuild the workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceState {
    pub roots: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PipelineStepInfo {
    pub index: usize,
    pub step: String,
    pub input: usize,
    pub output: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExporterInfo {
    pub id: String,
    pub name: String,
    pub extension: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    #[serde(default)]
    pub query: Query,
    pub format: String,
    pub path: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub format: String,
    pub path: String,
    pub events: usize,
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
    pub message: &'a str,
    pub template: Option<&'a str>,
    pub exception: Option<&'a str>,
    pub fields: &'a Map<String, Value>,
    pub source: u32,
    pub source_name: &'a str,
    pub line: u32,
    pub group: String,
    pub markers: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<'a> {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<RecordView<'a>>,
    pub anchor_index: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerCluster {
    pub kind: String,
    pub bucket: usize,
    pub ts: i64,
    pub id: u32,
    pub count: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lane {
    pub key: String,
    pub first: i64,
    pub last: i64,
    pub total: usize,
    pub matched: usize,
    pub density: Vec<u32>,
    pub markers: Vec<MarkerCluster>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub from: i64,
    pub to: i64,
    pub bucket_ms: f64,
    pub histogram: Vec<[u32; 6]>,
    pub lanes: Vec<Lane>,
    pub truncated_lanes: usize,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct FacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Facet {
    pub field: String,
    pub values: Vec<FacetValue>,
    pub distinct: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ChartSeries {
    pub key: String,
    pub points: Vec<(i64, Option<f64>)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleEvent {
    pub kind: String,
    pub ts: i64,
    pub group: String,
    pub id: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartData {
    pub def: ChartDef,
    pub from: i64,
    pub to: i64,
    pub bucket_ms: i64,
    pub series: Vec<ChartSeries>,
    pub lifecycle: Vec<LifecycleEvent>,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct ExtractSpan {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractSample {
    pub record_id: u32,
    pub text: String,
    pub spans: Vec<ExtractSpan>,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ExtractPreview {
    pub candidates: usize,
    pub matches: usize,
    pub failed: usize,
    pub samples: Vec<ExtractSample>,
    pub error: Option<String>,
}
