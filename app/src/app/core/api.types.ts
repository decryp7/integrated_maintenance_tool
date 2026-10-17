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
 *   export_events          { request: ExportRequest }      -> ExportResult  (built-in formats "csv" and "clef")
 *   source_context         { request: ContextRequest }     -> SourceContext (original lines around an event)
 *   read_file_head         { path: string, maxBytes?: number } -> string   (only files in the workspace)
 *   preview_parse          { request: ParsePreviewRequest } -> ParsePreview
 *   set_live               { enabled: boolean, options?: LiveOptions } -> number (watched folders; call again after opening more files)
 *   tail_status            {}                              -> TailStatus    (per-file live tail state and totals)
 *   live_state             {}                              -> LiveState     (live tail on/off and its effective options)
 *   mute_alerts            { target: AlertTarget, untilMs: number } -> void (not raised until untilMs; at most MAX_MUTE_MS ahead)
 *   unmute_alerts          { target: AlertTarget }         -> void
 *   open_paths_live        { paths: string[], startAtEnd?: boolean } -> WorkspaceSummary (startAtEnd: only lines written from now on)
 *   infer_regex            { lines: string[] }             -> InferredRegex
 *   explain_regex          { pattern: string }             -> ExplainNode[]
 *   annotations            { request: AnnotationsRequest } -> AnnotationDto[] (plugin annotator spans/points, by start time)
 *   plugin_events          {}                              -> SubscriberMetrics[] (event-bus delivery counters per subscriber)
 *
 * Analysis commands: see the "Analysis" section below. Error snapshot commands: see "Error snapshots".
 *
 * Events (Tauri): "imt://workspace-changed" (payload `TailReport`; listeners must accept none) after live tail changed the workspace
 * (at most about four per second); "imt://live-alert" (`LiveAlert`) for each alert a live refresh raised;
 * "imt://error" (EngineError) when the engine contained a new plugin crash, quarantine or import crash.
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
  /**
   * Logical log the file belongs to: rolled, dated and rotated files of one log share it
   * (`logs/app.log` for `logs/app.log.1`), so the UI can collapse them.
   */
  group: string;
  /** Importer id: "json", "text", "csv" or "registry" (plugins add more). */
  importer: string;
  records: number;
  /** Events removed from this source by the profile pipeline. */
  dropped: number;
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
  /** Who defines the kind: `profile`, `builtin` or a plugin id (plugin kinds are named `<plugin-id>/<kind>`). */
  owner: string;
  /** Lifecycle meaning of a plugin kind (start, stop, crash); null for others. */
  role: MarkerRole | null;
  label: string;
  /** Lifecycle markers (start/stop) can be shown on the timeline regardless of level/text filters. */
  lifecycle: boolean;
  color: string | null;
  /** A built-in or custom icon name; null = built-in shape. */
  icon: string | null;
  /** Generated badge text (1 to 3 characters); takes precedence over `icon`. */
  badge: string | null;
  /** Badge outline (the engine rejects any other value in the profile). */
  shape: MarkerShape | null;
  /** Meaning of the marker, for the legend and tooltips. */
  description: string | null;
  /**
   * The default rule of a plugin kind (all conditions must match), shown read-only. Empty for profile and
   * built-in kinds (their rules are in the profile) and for kinds a plugin only marks programmatically.
   */
  when: RuleCondition[];
}

/** One condition of a rule, in the profile condition language (`[[marker]] when`). */
export interface RuleCondition {
  field: string;
  op: string;
  value: string | number | boolean | string[] | null;
}

/** Lifecycle meaning of a plugin marker kind, for the timeline and charts. */
export type MarkerRole = 'start' | 'stop' | 'crash';

/** Owner of the marker kinds defined in the profile itself and of the engine's built-in kinds. */
export const PROFILE_OWNER = 'profile';
export const BUILTIN_OWNER = 'builtin';

export type MarkerShape = 'circle' | 'square' | 'diamond' | 'hexagon';
export const MARKER_SHAPES: readonly MarkerShape[] = ['circle', 'square', 'diamond', 'hexagon'];

/** A custom marker icon from the profile, sanitised by the engine so it is safe to render. */
export interface CustomIcon {
  name: string;
  svg: string | null;
  /** `data:image/png;base64,...` */
  pngDataUrl: string | null;
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
  /** Events removed by the profile pipeline, over all sources. */
  dropped: number;
  /** Records evicted by the live retention limit so far. */
  evicted: number;
  /** Backend plugins (importers, transforms, exporters, hooks) that are loaded. */
  plugins: PluginInfo[];
  /** Formats `export_events` can write. */
  exporters: ExporterInfo[];
  /** Custom marker icons from the profile, by name. */
  icons: CustomIcon[];
  /** Events in/out of each `[[pipeline]]` step during the last import, summed over sources. */
  pipeline: PipelineStat[];
}

export interface PipelineStat {
  /** Position of the step in the profile's `[[pipeline]]` array. */
  index: number;
  step: string;
  input: number;
  output: number;
}

export type PluginStatus = 'active' | 'disabled' | 'quarantined' | 'inactive';

/** A dependency from a plugin's manifest. */
export interface DepInfo {
  id: string;
  /** Semver requirement, e.g. `^1.0`. */
  req: string;
  optional: boolean;
}

/** A service a plugin provides to other plugins. */
export interface ServiceInfo {
  id: string;
  version: string;
}

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  /** What the plugin adds, e.g. "importer:registry", "transform:x", "exporter:y", "hook", "service:z". */
  contributes: string[];
  /**
   * `disabled`: turned off by the user or safe mode; `quarantined`: crashed repeatedly and is no longer called;
   * `inactive`: enabled but not running (see `inactiveReason`).
   */
  status: PluginStatus;
  /** Contained crashes/errors attributed to the plugin. */
  failures: number;
  /** The latest contained error, if any. */
  lastError: string | null;
  /** Dependencies from the manifest. */
  requires: DepInfo[];
  /** Services the plugin provides. */
  provides: ServiceInfo[];
  /** Why an `inactive` plugin is not running, e.g. `needs net.decryptology.perfmon ^1.0 (installed 0.9.0)`. */
  inactiveReason: string | null;
  /** Plugins that require this one (they stop when it stops). */
  dependents: string[];
  /** Non-fatal problems: unavailable optional dependencies, inactive contributions. */
  warnings: string[];
  /** A call is running past its time budget right now. */
  hung: boolean;
}

/** Delivery counters of one plugin's event subscription (`plugin_events`). */
export interface SubscriberMetrics {
  id: number;
  plugin: string;
  /** Rust type name of the event, e.g. `imt_core::events::RecordsAppended`. */
  event: string;
  delivered: number;
  /** Dropped because the subscriber's queue was full (oldest first). */
  dropped: number;
  /** Deliveries that failed (the handler panicked or overran its budget). */
  failures: number;
  /** Not delivered because the subscriber was quarantined, hung or inactive. */
  skipped: number;
  queued: number;
}

export interface ExporterInfo {
  id: string;
  name: string;
  /** File extension without the dot, used to pick the format from a file name. */
  extension: string;
}

export interface ExportRequest {
  query: Query;
  /** Exporter id, e.g. "csv" or "clef". */
  format: string;
  path: string;
  descending?: boolean;
}

export interface ExportResult {
  format: string;
  path: string;
  events: number;
}

/**
 * Fields of records produced by the "registry" importer (Windows .reg exports).
 * `Data` is a number for REG_DWORD/REG_QWORD, an unescaped string for string types and a hex string for binary.
 */
export interface RegistryFields {
  Hive: string;
  /** Full key path without the hive, e.g. `SOFTWARE\Contoso\OrderService`. */
  Key: string;
  /** "(Default)" for the unnamed value; absent on key lines. */
  ValueName?: string;
  ValueType?: 'REG_SZ' | 'REG_DWORD' | 'REG_QWORD' | 'REG_BINARY' | 'REG_MULTI_SZ' | 'REG_EXPAND_SZ' | 'DELETE';
  Data?: string | number;
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
  /** Query-language expression evaluated by the engine; default (absent or empty) = none. */
  ql?: string;
  /** A sliding time window the engine resolves into `from` (live tail); absent = none. */
  window?: TimeWindow | null;
}

/**
 * A time window relative to "now": the newest record (default) or the wall clock (`[live] clock = "wall"`).
 * The engine resolves it, so the UI never computes "now" itself.
 */
export type TimeWindow = { kind: 'live'; lastMs: number };

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
  template: string | null;
  exception: string | null;
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

export interface ContextRequest {
  recordId: number;
  /** Lines before the event (default 10; the engine caps the total). */
  before: number;
  after: number;
}

export interface SourceLine {
  line: number;
  text: string;
}

export interface SourceContext {
  sourceName: string;
  /** Line of the event itself. */
  line: number;
  lines: SourceLine[];
}

export interface ParsePreviewRequest {
  /** Candidate profile (may be unsaved). */
  toml: string;
  /** Sample content, e.g. the first lines of a file. */
  sample: string;
  fileName: string;
  limit: number;
}

export interface ParsedPreview {
  line: number;
  ts: number | null;
  level: Level;
  message: string;
  exception: string | null;
  fields: Record<string, unknown>;
  markers: string[];
}

export interface ParsePreview {
  /** Importer that read the sample; null when none did. */
  importer: string | null;
  records: ParsedPreview[];
  total: number;
  skipped: number;
  dropped: number;
  error: string | null;
}

// ---- Regex assistance -----------------------------------------------------------------------

/** What an inferred group captures (crates/imt-core/src/regex_assist.rs `InferredGroup.kind`). */
export type GroupKind = 'number' | 'decimal' | 'guid' | 'ipv4' | 'hex' | 'timestamp' | 'word' | 'token' | 'text';

export interface InferredGroup {
  name: string;
  kind: GroupKind;
  /** A few values the group captured from the example lines. */
  examples: string[];
}

/** One line of a regex explanation outline; start/end are UTF-8 byte offsets into the pattern. */
export interface ExplainNode {
  depth: number;
  text: string;
  start: number;
  end: number;
}

/** Result of `infer_regex`: a pattern with named, typed groups that matches the example lines. */
export interface InferredRegex {
  pattern: string;
  groups: InferredGroup[];
  /** Whether the pattern matches each input line. */
  matches: boolean[];
  explanation: ExplainNode[];
}

// ---- Analysis -------------------------------------------------------------------------------

/**
 * Analysis commands (query language, patterns, exceptions, anomalies, comparison,
 * field statistics, traces, requests, Moment view). Mirrors the Rust DTOs in
 * `crates/imt-core/src/{api.rs,ql,analysis,patterns.rs,exceptions.rs}`; camelCase on the wire.
 *
 *   run_query        { request: { query } }                 -> { result: QlResult } | { error: QlError }
 *   check_query      { text }                               -> QlError | null
 *   complete_query   { request: { text, cursor } }          -> Completion[]
 *   query_patterns   { request: { query } }                 -> Pattern[]
 *   query_exceptions { request: { query } }                 -> ExceptionGroup[]
 *   query_anomalies  { request: { query } }                 -> Anomaly[]
 *   compare_windows  { request: { a, b } }                  -> Comparison
 *   field_stats      { request: { query, field, buckets } } -> FieldStats
 *   correlate        { request: { field, value } }          -> number[]
 *   query_traces     { request: { query, traceId } }        -> { traces, waterfall }
 *   query_requests   { request: { query, idField } }        -> RequestSpan[]
 *   query_moment     { request: { query, from, to } }       -> MomentSummary
 *
 * Offsets in `QlError` and the `cursor` of `complete_query` are UTF-8 byte offsets.
 */

/** `Query` plus the query-language text (`Query.ql` on the Rust side; empty = no ql filter). */
export interface QlQuery extends Query {
  ql?: string;
}

export interface QlError {
  message: string;
  /** Byte offset into the query text. */
  offset: number;
  /** Length in bytes; 0 marks a position (e.g. "unexpected end of query"). */
  len: number;
}

export type QlResult =
  | { kind: 'events'; ids: number[]; fields: string[] | null }
  | { kind: 'table'; columns: string[]; rows: unknown[][] };

export type QueryOutcome = { result: QlResult } | { error: QlError };

export type CompletionKind = 'field' | 'keyword' | 'operator' | 'function' | 'value';

export interface Completion {
  label: string;
  kind: CompletionKind;
  /** Text that replaces the word being typed. */
  insert: string;
}

export interface Pattern {
  id: number;
  template: string;
  count: number;
  /** Count per level, in `LEVELS` order. */
  levels: number[];
  firstTs: number | null;
  lastTs: number | null;
  sampleIds: number[];
  source: 'Serilog' | 'Mined';
}

export interface ExceptionGroup {
  fingerprint: string;
  typeName: string;
  message: string;
  topFrame: string | null;
  count: number;
  /** [instance key, count] pairs. */
  instances: [string, number][];
  firstTs: number | null;
  lastTs: number | null;
  sampleIds: number[];
}

export type AnomalyKind = 'errorSpike' | 'silence' | 'crash' | 'slowOutlier' | 'rateChange' | 'memoryGrowth';

export interface Anomaly {
  kind: AnomalyKind;
  instance: string | null;
  from: number;
  to: number;
  score: number;
  message: string;
  sampleIds: number[];
}

export interface LevelCount {
  level: Level;
  count: number;
  perMinute: number;
}

export interface SideStats {
  count: number;
  from: number | null;
  to: number | null;
  durationMs: number;
  perMinute: number;
  levels: LevelCount[];
}

export interface ValueShare {
  value: string;
  countA: number;
  countB: number;
  shareA: number;
  shareB: number;
}

export interface FieldShift {
  field: string;
  /** Total variation distance between the two value distributions (0..1). */
  distance: number;
  top: ValueShare[];
}

export interface NumberSummary {
  count: number;
  median: number;
  p95: number;
}

export interface NumericShift {
  field: string;
  a: NumberSummary;
  b: NumberSummary;
  medianChangePct: number | null;
  p95ChangePct: number | null;
}

export interface ValueCount {
  value: string;
  count: number;
}

export interface ValueAppearance {
  field: string;
  values: ValueCount[];
}

export interface Comparison {
  a: SideStats;
  b: SideStats;
  changedFields: FieldShift[];
  numericChanges: NumericShift[];
  /** Values seen only in B. */
  newValues: ValueAppearance[];
  /** Values seen only in A. */
  goneValues: ValueAppearance[];
  summary: string[];
}

export interface HistogramBin {
  from: number;
  to: number;
  count: number;
}

export interface NumericStats {
  min: number;
  max: number;
  mean: number;
  stddev: number;
  p50: number;
  p90: number;
  p95: number;
  p99: number;
  histogram: HistogramBin[];
}

export interface FieldStats {
  count: number;
  missing: number;
  distinct: number;
  numeric: NumericStats | null;
  top: ValueCount[];
}

export interface TraceSummary {
  traceId: string;
  spans: number;
  errors: number;
  start: number;
  durationMs: number;
  root: string;
}

export interface WaterfallRow {
  spanId: string;
  parent: string | null;
  depth: number;
  name: string;
  startOffsetMs: number;
  durationMs: number;
  errors: number;
}

export interface TracesResult {
  traces: TraceSummary[];
  waterfall: WaterfallRow[];
}

export interface RequestSpan {
  id: string;
  first: number;
  last: number;
  events: number;
  errors: number;
  instance: string;
}

export interface MomentInstance {
  key: string;
  events: number;
  warnings: number;
  errors: number;
  patterns: Pattern[];
  /** First error or fatal event in the window. */
  firstError: number | null;
}

export interface MetricWindow {
  series: string;
  metric: string;
  samples: number;
  min: number;
  avg: number;
  max: number;
  last: number;
  /** Average over the window of the same length just before. */
  previousAvg: number | null;
  changePct: number | null;
}

export interface ActivityItem {
  id: number;
  ts: number;
  instance: string;
  user: string | null;
  action: string;
  level: Level;
}

/** How many events in a moment came from one user, and the field that named them. */
export interface UserCount {
  /** The profile's `activity.user` field the value was read from. */
  field: string;
  value: string;
  count: number;
}

export interface MomentSummary {
  from: number;
  to: number;
  total: number;
  levels: Partial<Record<Level, number>>;
  instances: MomentInstance[];
  metrics: MetricWindow[];
  users: UserCount[];
  activity: ActivityItem[];
  anomalies: Anomaly[];
}

// ---- Error snapshots ------------------------------------------------------------------------

/*
 * Mirrors app/src-tauri/src/snapshots.rs:
 *   save_error_snapshot    { image: base64, mime, context: SnapshotContext } -> SnapshotSaveResult
 *   list_error_snapshots   {}                     -> SnapshotInfo[]  (newest first)
 *   read_error_snapshot    { id }                 -> string (data: URL)
 *   delete_error_snapshot  { id }                 -> void
 *   open_snapshots_folder  {}                     -> string (folder path)
 *   export_diagnostics     { path, ids }          -> number (files written)
 *   diagnostics_settings   {}                     -> { snapshots: { enabled: boolean } }
 * Event "imt://error" carries an EngineError.
 */

/** Why a snapshot was taken. */
export type SnapshotReason = 'unhandled-error' | 'panel-failed' | 'backend-failed' | 'engine-error' | 'manual';

/** What the UI sends with an image. */
export interface SnapshotContext {
  reason: SnapshotReason;
  message: string;
  stack?: string;
  /** The active view's title. */
  view?: string;
  /** The user asked for it; allowed even when automatic snapshots are off. */
  manual: boolean;
}

/** Result of `save_error_snapshot` (Rust `SaveResult`). */
export interface SnapshotSaveResult {
  id: string;
  /** The first snapshot ever saved: show the "saved locally" notice. */
  firstTime: boolean;
}

/** One saved snapshot (newest first in listings). */
export interface SnapshotInfo {
  id: string;
  /** RFC 3339, UTC. */
  createdAt: string;
  reason: string;
  message: string;
  stack: string | null;
  view: string | null;
  manual: boolean;
  appVersion: string;
  os: string;
  mime: string;
  bytes: number;
}

/** `plugin-inactive`: a plugin stopped running (a dependency became unavailable or activation failed). */
export type EngineErrorKind = 'plugin-crash' | 'plugin-quarantined' | 'plugin-inactive' | 'import-crash';

/** Payload of the `imt://error` event: a failure the engine contained. */
export interface EngineError {
  kind: EngineErrorKind;
  message: string;
  /** The plugin concerned, when there is one. */
  plugin: string | null;
}

/** Name of the event the engine emits for contained failures. */
export const ENGINE_ERROR_EVENT = 'imt://error';
export type * from '../settings/settings.types';

// ---------------------------------------------------------------------------------------------
// Live tail (crates/imt-core/src/workspace/tail/types.rs, app/src-tauri/src/live.rs)
// ---------------------------------------------------------------------------------------------

/** How urgent a finding is (health check, live alerts). */
export type Severity = 'critical' | 'warning' | 'info';
export const SEVERITIES: readonly Severity[] = ['critical', 'warning', 'info'];

/** What happens to the records of a file that disappears while tailed. */
export type DeletedFiles = 'keep' | 'drop';

/** How the shell notices file changes: auto = events, polling for network paths and after watcher errors. */
export type WatchPreference = 'auto' | 'events' | 'polling';

/** How one file is actually watched. */
export type WatchMode = 'events' | 'polling' | 'off';

/** Bounded memory for long live sessions; both limits are off when null/absent. */
export interface Retention {
  /** Keep at most this many records (the newest). */
  maxRecords?: number | null;
  /** Keep only records at most this old, relative to the newest record. */
  maxAgeMs?: number | null;
}

/** Options of `set_live`; every key is optional (engine defaults apply). */
export interface LiveOptions {
  watch?: WatchPreference;
  /** Polling interval (clamped by the shell to 100 ms .. 10 min; default 2000). */
  pollIntervalMs?: number | null;
  deletedFiles?: DeletedFiles;
  retention?: Retention;
  /** An unterminated last line is parsed anyway after this long without a newline (default 2000). */
  carryFlushMs?: number;
  /** The live anomaly detectors run at most this often (default 1000; 0 .. MAX_DETECT_EVERY_MS, clamped by the engine). */
  detectEveryMs?: number;
}

/** Longest `detectEveryMs` the engine accepts (10 minutes). */
export const MAX_DETECT_EVERY_MS = 600_000;

/** Reply of `live_state`: whether the window's live tail runs, and the options in effect (defaults resolved, values clamped). */
export interface LiveState {
  enabled: boolean;
  options: LiveOptions;
}

/** A source whose file was renamed (rotation) and keeps its records. */
export interface RotatedSource {
  sourceId: number;
  from: string;
  to: string;
}

/** What one live refresh changed: the payload of `imt://workspace-changed`. */
export interface TailReport {
  /** New records (not counting records rewritten in place). */
  appended: number;
  /** Existing records rewritten in place (continuation lines merged into them). */
  updated: number;
  /** Sources whose records changed in any way. */
  sourcesChanged: number[];
  /** Sources created for files that appeared. */
  added: number[];
  rotated: RotatedSource[];
  /** Sources whose file was truncated or rewritten; the file is read again as a new source. */
  truncated: number[];
  /** Sources whose file disappeared (records kept unless `deletedFiles: 'drop'`). */
  deleted: number[];
  /** Sources read again in full (binary formats, archives, inconsistencies). */
  reimported: number[];
  /** Records removed by the retention limit. */
  evicted: number;
  /** Record ids at and after this one changed; null when every existing id is stable. */
  idsChangedFrom: number | null;
  /** Timestamp of the newest record. */
  newestTs: number | null;
  /** Instance keys seen for the first time. */
  instancesChanged: string[];
  /** Files whose live state changed in this refresh (new files included), for badges without polling `tail_status`. */
  fileStates: TailFileChange[];
  /** Number of `imt://live-alert` alerts raised by this refresh. */
  alertsRaised: number;
  elapsedMs: number;
}

/** A file whose live state changed in a refresh. */
export interface TailFileChange {
  sourceId: number;
  state: TailFileState;
}

/** Live state of one tailed file. */
export type TailFileState =
  | { kind: 'tailing' }
  /** Renamed by rotation; still read until the writer lets go. */
  | { kind: 'rotated' }
  /** Truncated or rewritten in place; its records stay, the file restarted as a new source. */
  | { kind: 'truncated' }
  /** Gone from disk; its records stay (or were dropped). */
  | { kind: 'deleted' }
  /** Read only as a whole (binary format or archive). */
  | { kind: 'whole' }
  | { kind: 'error'; message: string };

export interface TailFileStatus {
  sourceId: number;
  path: string;
  name: string;
  group: string;
  mode: WatchMode;
  /** Bytes turned into records. */
  offset: number;
  /** File size at the last read. */
  size: number;
  linesPerSec: number;
  /** Wall-clock time (ms) of the last change seen. */
  lastEvent: number | null;
  state: TailFileState;
}

export interface TailTotals {
  files: number;
  tailing: number;
  errors: number;
  bytes: number;
  linesPerSec: number;
  /** Records evicted by retention since the workspace was opened. */
  evicted: number;
  records: number;
}

export interface TailStatus {
  files: TailFileStatus[];
  totals: TailTotals;
}

/** A finding of the live detectors or of a user alert rule: the payload of `imt://live-alert`. */
export interface LiveAlert {
  /**
   * Stable id: the same finding has the same id (`kind:who:bucket`, `who` = rule name or instance, `bucket` = `from`
   * divided by the profile's `[live] cooldown`), so duplicates can be dropped.
   */
  id: string;
  /** An anomaly kind, or `rule` for a `[[live.rule]]`. */
  kind: AnomalyKind | 'rule';
  /** The alert rule's name for `rule`. */
  rule: string | null;
  instance: string | null;
  severity: Severity;
  from: number;
  to: number;
  message: string;
  /** The query that shows the evidence (time range, instance, levels). */
  evidenceQuery: Query;
}

/** What `mute_alerts` mutes: one anomaly kind or one alert rule (exactly one of the two). */
export type AlertTarget = { kind: AnomalyKind } | { rule: string };

/** Longest mute the engine accepts, from now (one day). */
export const MAX_MUTE_MS = 86_400_000;

/** Tauri event carrying one `LiveAlert`. */
export const LIVE_ALERT_EVENT = 'imt://live-alert';

// ---------------------------------------------------------------------------------------------
// Annotations (crates/imt-core/src/plugin/annotate.rs)
// ---------------------------------------------------------------------------------------------

/** Records with `field == value` (e.g. `Computer = WEB01`): a machine-wide range. */
export interface FieldMatch {
  field: string;
  value: string;
}

/** A point (`from == to`) or range a plugin annotator found, e.g. a reboot window or a service's uptime. */
export interface AnnotationDto {
  /** Id of the annotator that produced it. */
  annotator: string;
  /** Plugin id that contributed it. */
  owner: string;
  /** `<plugin-id>/<kind>`, usually one of the plugin's marker kinds (shares its style). */
  kind: string;
  from: number;
  to: number;
  /** The instance (timeline lane) it applies to; null = every instance, or those matching `on`. */
  instance: string | null;
  on: FieldMatch | null;
  label: string;
  severity: Severity;
  /** The query that shows the evidence. */
  evidenceQuery: Query;
}

/** Which annotations `annotations` returns; every key is optional. */
export interface AnnotationsRequest {
  /** Overlapping `[from, to]` (inclusive); open-ended when absent. */
  from?: number | null;
  to?: number | null;
  /** Only those for this instance (plus those for every instance). */
  instance?: string | null;
}
