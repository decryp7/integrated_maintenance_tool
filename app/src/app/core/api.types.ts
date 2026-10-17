/**
 * Contract between the Angular UI and the Rust backend (crates/imt-core, exposed
 * through Tauri commands in app/src-tauri). Field names are camelCase on the wire.
 *
 * Commands (invoke name -> args -> result):
 *   workspace_summary      {}                              -> WorkspaceSummary
 *   open_paths             { paths: string[] }             -> WorkspaceSummary
 *   remove_source          { sourceId: number }            -> WorkspaceSummary
 *   clear_workspace        {}                              -> WorkspaceSummary
 *   reload_workspace       {}                              -> WorkspaceSummary
 *   query_page             { request: PageRequest }        -> Page
 *   query_timeline         { request: TimelineRequest }    -> Timeline
 *   query_facet            { request: FacetRequest }       -> Facet
 *   query_chart            { request: ChartRequest }       -> ChartData
 *   get_profile            {}                              -> ProfileDoc
 *   validate_profile       { toml: string }                -> ProfileValidation
 *   save_profile           { toml: string }                -> WorkspaceSummary   (applies + re-imports)
 *   preview_extractor      { request: ExtractPreviewRequest } -> ExtractPreview
 *   import_profile         { path: string }                -> ProfileDoc   (read + validated; applied only by save_profile)
 *   export_profile         { path: string, toml: string }  -> void
 *
 * Timestamps are epoch milliseconds (UTC). Records whose timestamp could not be
 * determined have ts = null and never appear on the timeline or in charts.
 *
 * Pseudo-fields usable wherever a field name is accepted (filters, groupBy,
 * splitBy, conditions): "@level", "@message", "@exception", "@template",
 * "@source" (file name), "@line".
 */

export type Level = 'Verbose' | 'Debug' | 'Information' | 'Warning' | 'Error' | 'Fatal';
export const LEVELS: Level[] = ['Verbose', 'Debug', 'Information', 'Warning', 'Error', 'Fatal'];

export interface SourceInfo {
  id: number;
  path: string;
  name: string;
  /** Importer id: "json", "text" or "csv" (plugins add more). */
  importer: string;
  records: number;
  /** Lines/rows that could not be parsed. */
  skipped: number;
  error: string | null;
  from: number | null;
  to: number | null;
}

export interface UnrecognizedFile {
  path: string;
  name: string;
  reason: string;
}

export interface FieldInfo {
  name: string;
  /** Number of records carrying the field. */
  count: number;
  /** Distinct values (capped at 10000; 10000 means "10000 or more"). */
  distinct: number;
  /** True when every present value is numeric (chartable). */
  numeric: boolean;
  /** True when produced by a profile extractor rather than the log itself. */
  derived: boolean;
}

export interface MarkerKind {
  kind: string;
  label: string;
  /** Lifecycle markers (start/stop) can be shown on the timeline regardless of level/text filters. */
  lifecycle: boolean;
  color: string | null;
}

export type ChartType = 'line' | 'bar' | 'stacked';

export interface ChartDef {
  id: string;
  title: string;
  /** "count()", "avg(field)", "min(field)", "max(field)", "sum(field)", "last(field)". */
  y: string;
  /** "auto" or a duration like "30s", "1m", "5m", "1h". */
  bucket: string;
  splitBy: string[];
  type: ChartType;
  unit: string;
}

export interface WorkspaceSummary {
  sources: SourceInfo[];
  unrecognized: UnrecognizedFile[];
  total: number;
  from: number | null;
  to: number | null;
  levels: Record<Level, number>;
  fields: FieldInfo[];
  defaultGroupBy: string[];
  markerKinds: MarkerKind[];
  charts: ChartDef[];
  profileName: string;
  loadMs: number;
}

export type FilterOp = 'eq' | 'ne' | 'contains' | 'notContains' | 'matches' | 'exists' | 'missing' | 'gt' | 'lt';

export interface FieldFilter {
  field: string;
  op: FilterOp;
  value?: string;
}

export interface Query {
  from?: number | null;
  to?: number | null;
  /** Empty = all levels. */
  levels: Level[];
  /** Searches message, exception and template. */
  text: string;
  regex: boolean;
  caseSensitive: boolean;
  filters: FieldFilter[];
  /** Fields forming the instance key; empty = WorkspaceSummary.defaultGroupBy. */
  groupBy: string[];
  /** Only these instance keys (as produced with groupBy); empty = all. */
  groups: string[];
  /** Only records carrying one of these marker kinds; empty = no restriction. */
  markers: string[];
}

export interface PageRequest {
  query: Query;
  offset: number;
  limit: number;
  descending: boolean;
  /** When set, the backend ignores offset and returns the page containing this record id. */
  anchor?: number | null;
}

export interface RecordView {
  id: number;
  ts: number | null;
  level: Level;
  message: string;
  template?: string | null;
  exception?: string | null;
  fields: Record<string, unknown>;
  source: number;
  sourceName: string;
  line: number;
  /** Instance key under the query's groupBy, values joined with " · ". */
  group: string;
  /** Marker kinds, highest priority first. */
  markers: string[];
}

export interface Page {
  total: number;
  offset: number;
  rows: RecordView[];
  /** Position of the anchor within the full filtered result, or null if filtered out. */
  anchorIndex: number | null;
}

export interface TimelineRequest {
  query: Query;
  /** Horizontal resolution, usually the plot width in px / 2. Clamped to 10..2000. */
  buckets: number;
  alwaysShowLifecycle: boolean;
}

export interface MarkerCluster {
  kind: string;
  bucket: number;
  /** Timestamp of the first record in the cluster. */
  ts: number;
  /** Record id of the first record in the cluster (jump target). */
  id: number;
  count: number;
  message: string;
}

export interface Lane {
  key: string;
  first: number;
  last: number;
  total: number;
  matched: number;
  /** Matched records per bucket. */
  density: number[];
  markers: MarkerCluster[];
}

export interface Timeline {
  from: number;
  to: number;
  bucketMs: number;
  /** [bucket][levelIndex in LEVELS order] = count of matched records. */
  histogram: number[][];
  lanes: Lane[];
  truncatedLanes: number;
}

export interface FacetRequest {
  query: Query;
  field: string;
  limit: number;
}

export interface Facet {
  field: string;
  values: { value: string; count: number }[];
  distinct: number;
  total: number;
}

export interface ChartRequest {
  query: Query;
  /** Either the id of a chart from the profile, or an ad-hoc definition. */
  chartId?: string | null;
  def?: ChartDef | null;
  buckets: number;
}

export interface ChartSeries {
  key: string;
  /** [bucketStartMs, value or null when the bucket is empty]. */
  points: [number, number | null][];
}

export interface LifecycleEvent {
  kind: string;
  ts: number;
  group: string;
  id: number;
}

export interface ChartData {
  def: ChartDef;
  from: number;
  to: number;
  bucketMs: number;
  series: ChartSeries[];
  lifecycle: LifecycleEvent[];
}

export interface ProfileDoc {
  name: string;
  /** File the profile was loaded from; null for the built-in default. */
  path: string | null;
  toml: string;
}

export interface ProfileValidation {
  ok: boolean;
  error: string | null;
  /** 1-based line of the error when known. */
  line: number | null;
}

export interface ExtractPreviewRequest {
  /** Profile TOML (may be unsaved) and the name of the [[extract]] rule to test. */
  toml: string;
  extractor: string;
  query: Query;
  limit: number;
}

export interface ExtractSpan {
  name: string;
  /** UTF-16 offsets into ExtractSample.text. */
  start: number;
  end: number;
}

export interface ExtractSample {
  recordId: number;
  text: string;
  spans: ExtractSpan[];
  values: Record<string, unknown>;
}

export interface ExtractPreview {
  /** Records the rule's `when` conditions selected. */
  candidates: number;
  matches: number;
  failed: number;
  samples: ExtractSample[];
  error: string | null;
}
