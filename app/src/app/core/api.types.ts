/**
 * Contract between the Angular UI and the Rust backend (crates/imt-core, exposed
 * through Tauri commands in app/src-tauri). Field names are camelCase on the wire.
 *
 * Commands (invoke name -> args -> result):
 *   workspace_summary      {}                              -> WorkspaceSummary
 *   open_paths             { paths: string[] }             -> WorkspaceSummary
 *   remove_source          { sourceId: number }            -> WorkspaceSummary
 *   import_problems        {}                              -> DiagnosticRow[]
 *   evaluate_fix           { fix: QuickFix }               -> FixEvaluation
 *   index_status           {}                              -> IndexStatus
 *   clear_workspace        {}                              -> WorkspaceSummary
 *   reload_workspace       {}                              -> WorkspaceSummary
 *   query_page             { request: PageRequest }        -> Page
 *   query_timeline         { request: TimelineRequest }    -> Timeline
 *   query_facet            { request: FacetRequest }       -> Facet
 *   query_chart            { request: ChartRequest }       -> ChartData
 *   get_profile            {}                              -> ProfileDoc
 *   validate_profile       { toml: string }                -> ProfileValidation
 *   save_profile           { toml: string }                -> number   (checks and saves now; the re-import is that import job)
 *   preview_extractor      { request: ExtractPreviewRequest } -> ExtractPreview
 *   import_profile         { path: string }                -> ProfileDoc   (read + validated; applied only by save_profile)
 *   export_profile         { path: string, toml: string }  -> void
 *   export_events          { request: ExportRequest }      -> number   (an export job; built-in formats "csv" and "clef")
 *   source_context         { request: ContextRequest }     -> SourceContext (original lines around an event)
 *   read_file_head         { path: string, maxBytes?: number } -> string   (only files in the workspace)
 *   preview_parse          { request: ParsePreviewRequest } -> ParsePreview  (request.toml is a parser TOML, ADR 0026)
 *   set_live               { enabled: boolean, options?: LiveOptions } -> number (watched folders; call again after opening more files)
 *   tail_status            {}                              -> TailStatus    (per-file live tail state and totals)
 *   live_state             {}                              -> LiveState     (live tail on/off and its effective options)
 *   mute_alerts            { target: AlertTarget, untilMs: number } -> void (not raised until untilMs; at most MAX_MUTE_MS ahead)
 *   unmute_alerts          { target: AlertTarget }         -> void
 *   open_paths_live        { paths: string[], startAtEnd?: boolean } -> WorkspaceSummary (startAtEnd: only lines written from now on)
 *   infer_regex            { lines: string[] }             -> InferredRegex
 *   explain_regex          { pattern: string }             -> ExplainNode[]
 *   annotations            { request: AnnotationsRequest } -> AnnotationDto[] (plugin annotator spans/points and runs, by start time)
 *   query_runs             { request: RunsRequest }        -> RunsReply     (runs between lifecycle markers; see "Runs")
 *   preview_spans          { request: SpanPreviewRequest } -> SpanPreview   (the Runs editor's preview of a candidate rule)
 *   plugin_events          {}                              -> SubscriberMetrics[] (event-bus delivery counters per subscriber)
 *
 * Analysis commands: see the "Analysis" section below. Error snapshot commands: see "Error snapshots". Dump and UMDH
 * commands (`dumps_*`): see "Dumps". Parser library commands (`parsers_*`, `parser_*`): see "Parsers". Store pane and
 * SQL console commands (`store_*`, `sql_*`): see "The Store pane" at the end.
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
  /** Events cut at the importer's event size limit; the rest continues in a new event with `ContinuedFromLine` (the engine always sends it). */
  splitEvents?: number;
  error: string | null;
  from: number | null;
  to: number | null;
  /** Clock-skew correction applied to this source's timestamps, in ms (0 = none; the engine always sends it). See `set_source_offset`. */
  offsetMs?: number;
  /** How the source is stored on disk: what Open file / Open at line / Show in folder can do with it. */
  fileKind: FileKind;
  /** Many events without a readable timestamp (the engine sends null when the timestamps are fine). */
  timestampIssue?: TimestampIssue | null;
  /**
   * Display name of the file a profile `[[route]]` split this logical source from (ADR 0020); null for a file's
   * own source. Logical sources of one file share its `path`; removing one removes the file.
   */
  routedFrom?: string | null;
  /** Import problems of this source, one entry per kind (the engine always sends it, maybe empty). */
  problems?: SourceProblem[];
  /** Id of the library parser that read the source (ADR 0026); null or absent when none did. */
  parser?: string | null;
}

/**
 * What went wrong while reading a source (Rust `ingest::ProblemKind`): a line over the size limit, an event over the
 * size limit, a line that is not an event, events without a readable timestamp, a field value that does not fit its
 * declared type, bytes invalid in the file's encoding, a format no importer recognises, a damaged archive, an event
 * split at a parallel chunk boundary, a line that looks like an event but matches no format (kept with the event
 * above), events past the logical-source limit of `[[route]]`.
 */
export type SourceProblemKind =
  | 'lineTooLong'
  | 'eventTooLarge'
  | 'lineNotParsed'
  | 'timestampUnparsed'
  | 'fieldInvalid'
  | 'encoding'
  | 'unknownFormat'
  | 'truncatedArchive'
  | 'eventSplit'
  | 'unmatchedLine'
  | 'routeLimit';

/** One kind of import problem in one source (Rust `ingest::Problem`). */
export interface SourceProblem {
  kind: SourceProblemKind;
  count: number;
  /** 1-based line numbers of the first occurrences (at most 20). */
  sampleLines: number[];
}

/**
 * How a source is stored: a plain text file (lines are file lines), a file read by a binary importer (event log,
 * trace), a gzip file (lines are in the decompressed text) or a zip archive member (no file of its own).
 */
export type FileKind = 'text' | 'binary' | 'compressed' | 'archiveMember';

/** `open_source_file({ request })`: what to do with one of the window's sources (the engine resolves the path). */
export type FileAction = 'open' | 'openAtLine' | 'reveal';
export interface OpenFileRequest {
  sourceId: number;
  action: FileAction;
  /** The event's line (1-based); needed for `openAtLine`. */
  line?: number;
}
/** Reply of `open_source_file`: what opened the file (an editor, the default program, the file manager) and its path. */
export interface FileOpened {
  with: 'editor' | 'defaultApp' | 'fileManager';
  path: string;
}


/**
 * A source whose timestamps could not be read: more than 5 % of its events, or all of its first 20 (Rust
 * `imt_core::api::TimestampIssue`, decided by `imt_core::timestamp_issue`).
 */
export interface TimestampIssue {
  /** Events (lines that start an event) without a readable timestamp. */
  unparsedLines: number;
  totalLines: number;
  /** Up to three such lines, each cut to 120 characters. */
  examples: string[];
  /** The 1-based line number of each example (same order). */
  exampleLines: number[];
  /** A timestamp format the engine checked against those lines, if one reads them. */
  suggestion: TimestampSuggestion | null;
}

/** A strftime format for a parser's `[format] timestamp_formats`, verified by the engine to parse the sample lines. */
export interface TimestampSuggestion {
  format: string;
  /** The timestamp text of the first sample, as the format reads it. */
  sample: string;
  /** The parser lines that add the format: a `[format]` fragment (`[format]\ntimestamp_formats = ["%a %b %d %H:%M:%S %Y"]\n`). */
  parserSnippet: string;
  /** Sample lines (without a readable timestamp) the format reads. */
  parsedSamples: number;
  /** Sample lines the format was checked against. */
  checkedSamples: number;
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
  /** The profile's `[[dashboard]]`s. */
  dashboards: DashboardCfg[];
  /** Pages, facets and source context are redacted (the window's "Redacted view"). */
  redactedView: boolean;
  /** Events without a timestamp (counted in `total`; in the grid, not on the timeline). */
  untimed: number;
}

export interface PipelineStat {
  /** Position of the step in the profile's `[[pipeline]]` array. */
  index: number;
  step: string;
  input: number;
  output: number;
  /** Events the step failed on and left unchanged (`script` runtime errors and limits); absent means 0. */
  errors?: number;
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
  /** Redact with the profile's `[redact]` rules (a fresh salt per export). */
  redact?: boolean;
  /** Replace an existing file at `path`; without it the engine refuses with a `FILE_EXISTS` error (see `isFileExistsError`). */
  overwrite?: boolean;
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
  /** How a plain `text` matches (smart search); absent = `substring`. A regex or case-sensitive search is a substring search. */
  textMode?: TextMode;
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
  /** Only events from these source ids (`SourceInfo.id`); absent or empty = all sources. Unknown ids match nothing. */
  sources?: number[];
}

/**
 * How the search box text matches (Rust `query::TextMode`): `words` = every word, the last a prefix, answered from the
 * full-text index; `wordsOrSubstring` = also the text inside words (after `search_partial` found such partial matches).
 */
export type TextMode = 'substring' | 'words' | 'wordsOrSubstring';

/** How a row matched a smart search (Rust `ports::SearchMatch`). */
export type SearchMatch = 'words' | 'partial';

/** `search_partial({ query })` → the events holding the text only as a substring (Rust `ports::PartialMatches`); null when cancelled. */
export interface PartialMatches {
  /** "N more partial matches". */
  added: number;
  /** False when a limit stopped the scan. */
  complete: boolean;
}

// ---- Search everything (Rust `imt_core::search_all`; `search_everything({ request })` → `SearchAllReply | null`) ----

/** What the search everything palette asks: its text (words like smart search) and results per kind (0/absent = 8, at most 50). */
export interface SearchAllRequest {
  text: string;
  limit?: number;
}

/** A run of a snippet: a matched word (`mark`) or the text around it. */
export interface SnippetPart {
  text: string;
  mark: boolean;
}

/** An event holding every word; the snippet is of its message (or exception or template, when the words are there). */
export interface EventHit {
  id: number;
  ts: number | null;
  level: Level;
  /** The source's name. */
  source: string;
  snippet: SnippetPart[];
}

/** A source whose name or path holds the words (`id` is `SourceInfo.id`). */
export interface SourceHit {
  id: number;
  name: string;
  path: string;
  records: number;
  snippet: SnippetPart[];
}

/** A field whose name holds the words (`value` null), or a value of a field with few distinct values that does. */
export interface FieldHit {
  field: string;
  value: string | null;
  /** Events with the field (with the value). */
  count: number;
  snippet: SnippetPart[];
}

/** A marker kind whose name holds the words. */
export interface MarkerHit {
  kind: string;
  snippet: SnippetPart[];
}

/** An exception group whose type, message or top frame holds the words. */
export interface ExceptionHit {
  fingerprint: string;
  typeName: string;
  /** Events in the group. */
  count: number;
  /** The group's first sample event. */
  sample: number | null;
  /** `Type: message` with the matched words marked. */
  snippet: SnippetPart[];
}

/** Whose a note or knowledge file is. */
export type DocScope = 'project' | 'global';

/** A note or knowledge file holding the words; `event` is the event a note is about. */
export interface DocHit {
  id: string;
  scope: DocScope;
  title: string;
  event: number | null;
  snippet: SnippetPart[];
}

/** How far the search of one kind got: all of it, until the time budget ran out, or not at all (the store failed). */
export type KindOutcome = { kind: 'complete' } | { kind: 'timedOut' } | { kind: 'failed'; message: string };

/** The first `items` of `total` matches of a kind. */
export interface KindHits<T> {
  total: number;
  items: T[];
  outcome: KindOutcome;
}

/** Every kind's results, in the order the palette shows them. */
export interface SearchAllReply {
  events: KindHits<EventHit>;
  sources: KindHits<SourceHit>;
  fields: KindHits<FieldHit>;
  markers: KindHits<MarkerHit>;
  exceptions: KindHits<ExceptionHit>;
  notes: KindHits<DocHit>;
  knowledge: KindHits<DocHit>;
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

/**
 * Where an event's timestamp comes from: read from the event (`parsed`), taken over from the event before it in the
 * same file (`inherited`), or none at all (`untimed`: `ts` is null; the event is in the grid, not on the timeline).
 */
export type TsState = 'parsed' | 'inherited' | 'untimed';

export interface RecordView {
  id: number;
  ts: number | null;
  tsState: TsState;
  level: Level;
  message: string;
  template: string | null;
  exception: string | null;
  fields: Record<string, unknown>;
  source: number;
  sourceName: string;
  line: number;
  /** Byte offset of the event's first line in its file. */
  byteOffset: number;
  /** Instance key under the query's groupBy, values joined with " · ". */
  group: string;
  /** Marker kinds, highest priority first. */
  markers: string[];
  /** Set on pages of a smart search: a word match or a partial (substring) match. */
  searchMatch?: SearchMatch;
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
  /** Ids of the sources with events in the lane, ascending (at most MAX_LANE_SOURCES). */
  sources: number[];
}

/** Most source ids a timeline lane lists (`Lane.sources`). */
export const MAX_LANE_SOURCES = 16;

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
  /** Candidate parser TOML (may be unsaved); empty = the parser the library would choose for the sample. */
  toml: string;
  /** Candidate profile TOML (may be unsaved); empty or absent = the active profile. */
  profile?: string;
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
  /** Id of the library parser that read the sample (ADR 0026); null for a candidate parser or none. */
  parser?: string | null;
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
 *   classify_search  { text, fields }                       -> SearchInputKind
 *   query_patterns   { request: { query } }                 -> Analyzed<Pattern[]>
 *   query_exceptions { request: { query } }                 -> Analyzed<ExceptionGroup[]>
 *   query_anomalies  { request: { query } }                 -> Analyzed<Anomaly[]>
 *   compare_windows  { request: { a, b } }                  -> Analyzed<Comparison>
 *   field_stats      { request: { query, field, buckets } } -> FieldStats
 *   correlate        { request: { field, value } }          -> Analyzed<number[]>
 *   query_traces     { request: { query, traceId } }        -> { traces, waterfall, coverage }
 *   query_requests   { request: { query, idField } }        -> Analyzed<RequestSpan[]>
 *   query_moment     { request: { query, from, to } }       -> Analyzed<MomentSummary>
 *
 * Analyses return how much of the matching data they looked at with their result (`Analyzed`, `Coverage`).
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
  /** "Did you mean": closest known names for an unknown field, best first (absent or empty when none). */
  suggestions?: string[];
}

export type QlResult =
  | { kind: 'events'; ids: number[]; fields: string[] | null }
  | { kind: 'table'; columns: string[]; rows: unknown[][] };

export type QueryOutcome = { result: QlResult } | { error: QlError };

/**
 * What the filter bar's one search field holds (`ql::InputKind`): words to find in messages, or a query-language
 * query. The engine decides (`classify_search`); `fields` are the workspace's field names.
 */
export type SearchInputKind = 'text' | 'query';

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
  coverage: Coverage;
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
 *   capture_window_image   { userRequested? }     -> WindowCapture  (app/src-tauri/src/window_capture, ADR 0034)
 *   save_error_snapshot   { image: base64, mime, context: SnapshotContext } -> SnapshotSaveResult
 *   list_error_snapshots   {}                     -> SnapshotInfo[]  (newest first)
 *   read_error_snapshot    { id }                 -> string (data: URL)
 *   delete_error_snapshot  { id }                 -> void
 *   open_snapshots_folder  {}                     -> string (folder path)
 *   export_diagnostics     { path, ids }          -> number (the export job writing the zip)
 *   diagnostics_settings   {}                     -> { snapshots: { enabled: boolean } }
 * Event "imt://error" carries an EngineError.
 *
 * Problem reports and the activity log (app/src-tauri/src/activity/report.rs):
 *   problem_report_preview { description? }               -> ProblemReportPreview
 *   save_problem_report    { path, description?, overwrite? } -> number (the export job writing the zip)
 *   open_activity_log      {}                              -> string (label of the new window showing IMT's own logs,
 *                                                             filtered to SourceContext = "IMT.Activity")
 */

export type * from '../diagnostics/diagnostics-report.types';

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

/**
 * Result of `capture_window_image` (Rust `window_capture::WindowCapture`): a native picture of the calling window's
 * webview, bounded (at most 1600 px wide, PNG or JPEG) and ready for `save_error_snapshot`, or why there is none.
 */
export type WindowCapture =
  | {
      status: 'captured';
      /** Base64, without the `data:` prefix. */
      image: string;
      mime: 'image/png' | 'image/jpeg';
      bytes: number;
    }
  | { status: 'unavailable'; reason: string };

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
  /** The operations leading up to the error (empty for snapshots taken before the activity log existed). */
  breadcrumbs: SnapshotBreadcrumb[];
  /** Human-readable steps to reproduce, generated from `breadcrumbs` by the engine. */
  steps: string[];
}

/** One activity entry attached to a snapshot or report (Rust `activity::Breadcrumb`). */
export interface SnapshotBreadcrumb {
  time: string;
  operation: string;
  trigger: string;
  operationId?: string;
  outcome?: 'ok' | 'cancelled' | 'error';
  errorCode?: string;
  durationMs?: number;
  view?: string;
  window: string;
  /** Absent when empty. */
  params?: Record<string, string>;
}

/** One file of a problem report, as previewed before saving (Rust `ReportEntry`). */
export interface ReportEntry {
  /** Path inside the zip, e.g. `logs/activity.log`. */
  name: string;
  bytes: number;
  summary: string;
}

/** What a problem report would contain (Rust `ProblemReportPreview`). Nothing is sent anywhere. */
export interface ProblemReportPreview {
  entries: ReportEntry[];
  steps: string[];
  /** OS, version, build commit, session id, memory, plugins and settings (secrets removed). */
  systemInfo: Record<string, unknown>;
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

/**
 * A point (`from == to`) or range a plugin annotator found, e.g. a reboot window or a service's uptime — or a run
 * between lifecycle markers of the profile's `[[span]]` rules (owner "profile", with `run`).
 */
export interface AnnotationDto {
  /** Id of the annotator that produced it. */
  annotator: string;
  /** Plugin id that contributed it ("profile" for runs). */
  owner: string;
  /** `<plugin-id>/<kind>`, usually one of the plugin's marker kinds (shares its style); the `[[span]]` kind for runs. */
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
  /** Present on runs between lifecycle markers only. */
  run?: RunInfo;
}

/** Which annotations `annotations` returns; every key is optional. */
export interface AnnotationsRequest {
  /** Overlapping `[from, to]` (inclusive); open-ended when absent. */
  from?: number | null;
  to?: number | null;
  /** Only those for this instance (plus those for every instance). */
  instance?: string | null;
}

// ---------------------------------------------------------------------------------------------
// Runs between lifecycle markers (crates/imt-core/src/spans): `[[span]]` rules pair start and stop
// markers per timeline lane. Runs are annotations (owner "profile", annotator RUN_ANNOTATOR) with `run`.
//   query_runs     { request: RunsRequest }        -> RunsReply   (sorted page + summaries per span kind)
//   preview_spans  { request: SpanPreviewRequest } -> SpanPreview (the Runs editor's live preview)
// ---------------------------------------------------------------------------------------------

/** Annotator id of runs. */
export const RUN_ANNOTATOR = 'profile.spans';

/** How a run ended: started and stopped; still running at the end of the data; started again without a stop; a stop without a start (a point). */
export type RunStatus = 'closed' | 'open' | 'noStop' | 'noStart';
export const RUN_STATUSES: readonly RunStatus[] = ['closed', 'open', 'noStop', 'noStart'];

/** The start or stop event of a run (message cut to 240 characters). */
export interface RunEvent {
  id: number;
  ts: number;
  message: string;
}

/** What a run annotation adds to `AnnotationDto`. */
export interface RunInfo {
  /** The `[[span]]` kind. */
  span: string;
  spanLabel: string;
  status: RunStatus;
  start: RunEvent | null;
  stop: RunEvent | null;
  /** `to - from` (0 for a stop without start). */
  durationMs: number;
  /** Longer than the rule's `max_duration`. */
  tooLong: boolean;
  /** The field value it was paired by (`per = "field:<Name>"`). */
  group: FieldMatch | null;
  color: string | null;
}

/** Counts and durations of one span kind's runs over a selection. */
export interface SpanSummary {
  kind: string;
  label: string;
  color: string | null;
  enabled: boolean;
  runs: number;
  closed: number;
  open: number;
  noStop: number;
  noStart: number;
  tooLong: number;
  lanes: number;
  /** Over runs with a duration (all but stops without start). */
  totalMs: number;
  avgMs: number;
  maxMs: number;
  /** Runs dropped by the per-lane cap (1,000) plus events ignored by the group cap. */
  dropped: number;
  /** Marker kinds the rule names that are not defined (or switched off). */
  missingMarkers: string[];
}

export type RunSort = 'start' | 'stop' | 'duration' | 'status' | 'lane';

/** Which runs `query_runs` returns; every key is optional. */
export interface RunsRequest {
  /** Overlapping `[from, to]` (inclusive). */
  from?: number | null;
  to?: number | null;
  /** Only these lanes; empty = every lane. */
  instances?: string[];
  /** Only this span kind. */
  kind?: string | null;
  statuses?: RunStatus[];
  sort?: RunSort;
  descending?: boolean;
  offset?: number;
  /** Page size (default 200, at most 1000). */
  limit?: number | null;
}

/** A `[[span]]` table as the profile writes it (snake_case keys; default_profile.toml documents them). */
export interface SpanCfgToml {
  kind: string;
  label?: string;
  start: string | string[];
  stop: string | string[];
  /** "instance" or "field:<Name>". */
  per?: string;
  color?: string;
  max_duration?: string;
  restart_on_start?: boolean;
  enabled?: boolean;
}

export interface RunsReply {
  /** Every span kind (switched-off rules too, without runs), over the selection. */
  summaries: SpanSummary[];
  /** The profile's `[[span]]` rules as written (switched-off ones too). */
  rules: SpanCfgToml[];
  /** Run annotations (each with `run`), sorted as requested. */
  runs: AnnotationDto[];
  total: number;
  offset: number;
}

/** What starts or stops a run in the Runs editor: existing marker kinds, or a text pattern (saving creates a marker rule). */
export type SpanTrigger = { type: 'markers'; kinds: string[] } | { type: 'pattern'; pattern: string; caseSensitive?: boolean };

/** A candidate `[[span]]` rule. `per` is "instance" or "field:<Name>". */
export interface SpanDraft {
  kind: string;
  label: string;
  start: SpanTrigger;
  stop: SpanTrigger;
  per: string;
  maxDuration?: string | null;
  restartOnStart?: boolean | null;
  color?: string | null;
}

export interface SpanPreviewRequest {
  draft: SpanDraft;
}

export type SpanField = 'kind' | 'label' | 'start' | 'stop' | 'per' | 'color' | 'maxDuration' | 'general';

export interface SpanIssue {
  field: SpanField;
  message: string;
}

/** What saving a draft writes into the profile (TOML-shaped, snake_case keys). */
export interface SpanPlan {
  /** The draft's `[[span]]` table. */
  span: SpanCfgToml;
  /** Every `[[span]]` table of the profile after saving (the draft replacing a rule of its kind, else added last). */
  spans: SpanCfgToml[];
  /** `[[marker]]` rules its text patterns need; each replaces one of the same kind. */
  markers: (Record<string, unknown> & { kind: string })[];
}

export interface SpanPreview {
  /** Problems that prevent saving, by form field. */
  errors: SpanIssue[];
  warnings: string[];
  /** Over the loaded data; null while there are errors. */
  summary: SpanSummary | null;
  /** The first 20 runs by start time. */
  runs: AnnotationDto[];
  /** The first 20 anomalies (no start, no stop, too long). */
  anomalies: AnnotationDto[];
  plan: SpanPlan | null;
}

// ---- Clock-skew correction (`set_source_offset`, `suggest_offsets`) ----

/** How a source's new offset is given: an explicit value, or two events that happened at the same moment. */
export type OffsetSpec =
  | { kind: 'manual'; source: number; offsetMs: number }
  /** Moves the target event's source so the target shows at the reference event's time. */
  | { kind: 'align'; reference: number; target: number };

/** `set_source_offset` request: the spec plus record ids whose new ids the UI needs (selection, bookmarks; at most 10 000). */
export type SetOffsetRequest = OffsetSpec & { track?: number[] };

/** What `set_source_offset` changed. */
export interface OffsetChange {
  source: number;
  offsetMs: number;
  previousMs: number;
  /** Ids at and after this one were renumbered (records moved in time); null = none. */
  idsChangedFrom: number | null;
  /** New id of each tracked id, in request order (null: it no longer exists). */
  tracked: (number | null)[];
}

/** `suggest_offsets` request; every key is optional. */
export interface SuggestRequest {
  /** Correlation-id fields; default CorrelationId, TraceId, RequestId. */
  fields?: string[];
  /** Source the others are aligned to; default: the best connected one. */
  reference?: number | null;
  /** Pairs further apart are ignored (default 3 600 000 = 1 h). */
  maxDeltaMs?: number;
  /** Pairs within this distance of the median agree (default 1000). */
  toleranceMs?: number;
}

export interface OffsetSuggestion {
  source: number;
  reference: number;
  currentMs: number;
  /** Absolute offset that aligns the source with the reference. */
  suggestedMs: number;
  pairs: number;
  agreeing: number;
  /** Median absolute deviation of the paired deltas. */
  spreadMs: number;
  /** 0..1: share of agreeing pairs, scaled down below ten pairs. */
  confidence: number;
}

export interface SuggestResult {
  reference: number | null;
  fields: string[];
  suggestions: OffsetSuggestion[];
  /** Sources sharing too few ids with the reference for a suggestion. */
  insufficient: number[];
}

// ---------------------------------------------------------------------------------------------
// Profiling: CPU/allocation profiles and heap snapshots (crates/imt-profile/src/api.rs,
// commands in app/src-tauri/src/profiling.rs). Per window. Ids are capture ids from
// `profiling_open`. Times inside a profile are nanoseconds since its start; `*Ms` are Unix ms.
// Every request is validated in Rust; commands reject with a `ProfilingError` ({ code, message }).
// ---------------------------------------------------------------------------------------------

/** What kind of failure a profiling command reports (act on the code, show the message). */
export type ProfilingErrorCode = 'notACapture' | 'io' | 'limit' | 'unsupported' | 'invalid' | 'notFound';

/** The rejection of every profiling command. */
export interface ProfilingError {
  code: ProfilingErrorCode;
  message: string;
}

/** `profiling_sniff({ paths })` → SniffResult[]: what a file is, from its first 64 KiB, without opening it. */
export type SniffKind = 'capture' | 'heap' | 'none';

export interface SniffResult {
  /** The path as passed. */
  path: string;
  kind: SniffKind;
}

/** What a metric counts. `calls` and `maxCallNs` are per node (not inclusive). */
export type MetricId = 'samples' | 'timeNs' | 'calls' | 'allocCount' | 'allocBytes' | 'exceptions' | 'waitNs' | 'maxCallNs' | 'liveBytes';
/** `sampled` values are statistical estimates (label them so); `exact` were all recorded. */
export type Measurement = 'sampled' | 'exact';
export type MetricUnit = 'count' | 'nanoseconds' | 'bytes';
export type ProfileKind = 'cpu' | 'alloc' | 'wall' | 'mixed';
export type FrameKind = 'managed' | 'native' | 'kernel' | 'unknown';
export type ProfileEventKind = 'gc' | 'jit' | 'exception' | 'lockWait' | 'wait' | 'marker';
/** Values per metric; only the profile's metrics are present. */
export type MetricValues = Partial<Record<MetricId, number>>;

export interface MetricDto {
  id: MetricId;
  measurement: Measurement;
  unit: MetricUnit;
  label: string;
}

export interface ProfileThread {
  /** Index within the profile (use in `view.threads`). */
  id: number;
  osId: number | null;
  name: string;
}

export interface ProfileSummary {
  capture: 'profile';
  id: number;
  name: string;
  path: string | null;
  /** Importer id: speedscope, folded, cpuprofile, chrome-trace, pprof, perf-script, nettrace, ... */
  format: string;
  kind: ProfileKind;
  /** Wall-clock start; null when the format does not record it. */
  startedMs: number | null;
  durationNs: number;
  /** Samples have timestamps: thread timeline and time ranges work. */
  hasTime: boolean;
  sampleCount: number;
  frameCount: number;
  eventCount: number;
  metrics: MetricDto[];
  defaultMetric: MetricId;
  threads: ProfileThread[];
  eventKinds: ProfileEventKind[];
  /** Import notes (truncations, unresolved addresses, skipped records). */
  notes: string[];
}

export interface HeapSummary {
  capture: 'heap';
  id: number;
  name: string;
  path: string | null;
  format: string;
  takenMs: number | null;
  typeCount: number;
  objectCount: number;
  totalBytes: number;
  /** Object graph present: retained sizes, dominators, paths to roots. */
  hasGraph: boolean;
  notes: string[];
}

/** `profiling_open({ path })`, `profiling_list()` → CaptureSummary[]. */
export type CaptureSummary = ProfileSummary | HeapSummary;

/** `profiling_spans()`: captures with a wall-clock time, for the log timeline (heap: fromMs == toMs). */
export interface ProfileSpan {
  id: number;
  name: string;
  capture: 'profile' | 'heap';
  kind: ProfileKind | null;
  fromMs: number;
  toMs: number;
}

/** Inclusive range in ns since the profile's start. */
export interface ProfileTimeRange {
  fromNs: number;
  toNs: number;
}

export type RecursionMode = 'off' | 'direct' | 'all';

export interface JustMyCode {
  enabled?: boolean; // default true
  /** `*` wildcard, case-insensitive, matched against module and function (max 64, 256 chars each). */
  frameworkPatterns?: string[];
  /** Also use the importer's framework flag on frames (default true). */
  useFrameFlags?: boolean;
}

/** Filters/transforms shared by every view of a profile; all keys optional. */
export interface ProfileView {
  range?: ProfileTimeRange | null;
  threads?: number[] | null;
  justMyCode?: JustMyCode | null;
  recursion?: RecursionMode;
}

export interface FrameDto {
  id: number;
  name: string;
  module: string | null;
  file: string | null;
  line: number | null;
  kind: FrameKind;
  isFramework: boolean;
}

/** `avgPerCall`: the metric's total divided by calls (needs call counts; rows without calls sort last). */
export type ValueOf = 'self' | 'total' | 'avgPerCall';
export type ProfileGroupBy = 'function' | 'type' | 'namespace' | 'module' | 'thread';
export type TreeDirection = 'topDown' | 'bottomUp';

/** `metric: null` sorts by name. `descending` defaults to true for metrics, false for names. */
export interface ProfileSortSpec {
  metric?: MetricId | null;
  of?: ValueOf;
  descending?: boolean | null;
}

/** `profiling_hotspots({ request })` → HotspotsPage. Default sort: self of the metric, descending. */
export interface HotspotsRequest {
  profileId: number;
  view?: ProfileView;
  groupBy?: ProfileGroupBy;
  /** Metric for percentages (default: the profile's defaultMetric). */
  metric?: MetricId | null;
  sort?: ProfileSortSpec | null;
  search?: string | null;
  offset?: number;
  /** 1..10000, default 200. */
  limit?: number;
}

export interface HotspotRow {
  key: string;
  name: string;
  module: string | null;
  /** Function grouping only. */
  frame: FrameDto | null;
  /** Thread grouping only. */
  thread: number | null;
  self: MetricValues;
  total: MetricValues;
  selfPct: number;
  totalPct: number;
  avgPerCallNs: number | null;
  maxPerCallNs: number | null;
}

export interface HotspotsPage {
  rows: HotspotRow[];
  totalRows: number;
  grandTotal: MetricValues;
  metric: MetricId;
  measurement: Measurement;
  samplesInView: number;
}

/** `profiling_tree({ request })` → TreePage: children of the node at `path` (frame ids from the top). */
export interface TreeRequest {
  profileId: number;
  view?: ProfileView;
  direction?: TreeDirection;
  path?: number[];
  metric?: MetricId | null;
  offset?: number;
  limit?: number;
}

export interface TreeNode {
  frame: FrameDto;
  self: MetricValues;
  total: MetricValues;
  selfPct: number;
  totalPct: number;
  hasChildren: boolean;
}

export interface TreePage {
  /** Sorted by the metric's total, descending. */
  nodes: TreeNode[];
  totalChildren: number;
  grandTotal: MetricValues;
  metric: MetricId;
  measurement: Measurement;
}

/** `profiling_hot_path({ request })` → HotPath ("expand hottest path"). */
/** `profiling_frame_paths({ request })` → FramePaths: hottest top-down paths to a function. */
export interface FramePathsRequest {
  profileId: number;
  view?: ProfileView;
  /** A frame id; every frame of the same function (name and module) counts. */
  frame: number;
  metric?: MetricId | null;
  /** Default 5, at most 50. */
  limit?: number | null;
}

export interface FramePath {
  /** Frame ids from the root: a call-tree path and a flame graph focus. */
  path: number[];
  frames: FrameDto[];
  value: number;
  /** Share of the view total (percent). */
  pct: number;
}

export interface FramePaths {
  /** Hottest first; nested recursive occurrences are part of the outer one. */
  paths: FramePath[];
  /** Occurrences before the limit; 0 when the function is not in this view. */
  totalPaths: number;
  metric: MetricId;
}

export interface HotPathRequest {
  profileId: number;
  view?: ProfileView;
  direction?: TreeDirection;
  path?: number[];
  metric?: MetricId | null;
  /** Stop below this share of the grand total (0..1, default 0.01). */
  minFraction?: number | null;
}

export interface HotPath {
  /** Full path from the top (request path + expansion). */
  path: number[];
  nodes: TreeNode[];
}

/** `profiling_butterfly({ request })` → Butterfly (callers/callees of a function). */
export interface ButterflyRequest {
  profileId: number;
  view?: ProfileView;
  frame: number;
  metric?: MetricId | null;
  limit?: number;
}

export interface ButterflyRow {
  /** null = the function was at the top of the stack. */
  frame: FrameDto | null;
  values: MetricValues;
  /** Share of the function's total. */
  pct: number;
}

export interface Butterfly {
  function: HotspotRow;
  callers: ButterflyRow[];
  callees: ButterflyRow[];
  metric: MetricId;
}

export type FlameOrder = 'name' | 'value';

/** Visible part of the focused node's width: 0 ≤ start < end ≤ 1. */
export interface FlameViewport {
  start: number;
  end: number;
}

/** `profiling_flame({ request })` → FlameGraph. Draw depth upwards (flame) or downwards (icicle). */
export interface FlameRequest {
  profileId: number;
  view?: ProfileView;
  direction?: TreeDirection;
  /** Path of the zoomed node; empty = everything. */
  focus?: number[];
  viewport?: FlameViewport;
  widthPx: number;
  /** Default 0.5. */
  minWidthPx?: number | null;
  maxDepth?: number | null;
  metric?: MetricId | null;
  order?: FlameOrder;
  search?: string | null;
  searchRegex?: boolean;
  /** Default 50000, max 200000. */
  maxRects?: number | null;
}

export interface FlameRect {
  frame: number;
  /** Index of the enclosing rectangle in `rects`; null at depth 0. */
  parent: number | null;
  /** 0 = focus level. */
  depth: number;
  /** Fractions of the drawing width, viewport applied (may extend past 0..1 at the edges). */
  x: number;
  w: number;
  value: number;
  selfValue: number;
  matched: boolean;
  /** Differential only: target share − baseline share (−1..1); colour red > 0, blue < 0. */
  delta?: number;
  /** Differential only: baseline value scaled to the target total. */
  baselineValue?: number;
}

export interface FlameGraph {
  rects: FlameRect[];
  frames: FrameDto[];
  /** Frames of the request's `focus` path, in order (zoom breadcrumbs). */
  focusFrames: FrameDto[];
  focusValue: number;
  grandTotal: number;
  matchedValue: number;
  matchedPct: number;
  truncated: boolean;
  maxDepth: number;
  metric: MetricId;
  measurement: Measurement;
}

/** `profiling_diff_flame({ request })` → FlameGraph with `delta`/`baselineValue` (top-down, target layout). */
export interface DiffFlameRequest {
  baselineId: number;
  targetId: number;
  view?: ProfileView;
  focus?: number[];
  viewport?: FlameViewport;
  widthPx: number;
  minWidthPx?: number | null;
  maxDepth?: number | null;
  metric?: MetricId | null;
  order?: FlameOrder;
  search?: string | null;
  searchRegex?: boolean;
  maxRects?: number | null;
}

export type DiffSort = 'absDeltaSelf' | 'absDeltaTotal' | 'deltaSelf' | 'deltaTotal' | 'name';
export type DiffStatus = 'new' | 'removed' | 'changed' | 'same';

/** `profiling_function_diff({ request })` → FunctionDiffPage. Shares are normalised per profile. */
export interface FunctionDiffRequest {
  baselineId: number;
  targetId: number;
  view?: ProfileView;
  groupBy?: ProfileGroupBy;
  metric?: MetricId | null;
  sort?: DiffSort;
  search?: string | null;
  offset?: number;
  limit?: number;
}

export interface DiffSide {
  self: number;
  total: number;
  selfPct: number;
  totalPct: number;
}

export interface FunctionDiffRow {
  key: string;
  name: string;
  module: string | null;
  targetFrame: number | null;
  baseline: DiffSide;
  target: DiffSide;
  /** Percentage points. */
  deltaSelfPct: number;
  deltaTotalPct: number;
  status: DiffStatus;
}

export interface FunctionDiffPage {
  rows: FunctionDiffRow[];
  totalRows: number;
  baselineTotal: number;
  targetTotal: number;
  metric: MetricId;
}

/** `profiling_threads({ request })` → ThreadTimeline. Error when the profile has no timestamps. */
export interface ThreadTimelineRequest {
  profileId: number;
  /** range and threads apply. */
  view?: ProfileView;
  /** 1..4000, default 400. */
  buckets?: number;
  /** Default samples (else the default metric). */
  metric?: MetricId | null;
  eventKinds?: ProfileEventKind[] | null;
  maxEvents?: number | null;
}

export interface ThreadLane {
  thread: ProfileThread;
  total: number;
  buckets: number[];
}

export interface ProfileEventDto {
  kind: ProfileEventKind;
  thread: number | null;
  startNs: number;
  durNs: number;
  label: string;
}

export interface ThreadTimeline {
  fromNs: number;
  toNs: number;
  bucketNs: number;
  /** Busiest first. */
  threads: ThreadLane[];
  events: ProfileEventDto[];
  eventsTruncated: boolean;
  metric: MetricId;
  startedMs: number | null;
}

/** Percent of the view's total unless noted; all optional. */
export interface FindingThresholds {
  topSelfPct?: number; // 5
  topSelfCount?: number; // 5
  gcPct?: number; // 10
  lockPct?: number; // 5
  exceptionCount?: number; // 1
  exceptionPct?: number; // 1
  /** Percentage points of growth. */
  regressionPct?: number; // 5
  starvationBlockedPct?: number; // 50
}

/** `profiling_findings({ request })` → ProfileFinding[] (most severe first). */
export interface FindingsRequest {
  profileId: number;
  baselineId?: number | null;
  view?: ProfileView;
  thresholds?: FindingThresholds;
}

export type ProfileFindingKind =
  | 'topSelf'
  | 'gcPressure'
  | 'lockContention'
  | 'exceptionHotPath'
  | 'threadPoolStarvation'
  | 'regression'
  | 'hotPath'
  | 'blocking'
  | 'jitTime'
  | 'allocations'
  | 'hotNativeFunction'
  | 'waitHotSpot'
  | 'nativeHeapGrowth';
export type FindingSeverity = 'info' | 'warning' | 'critical';
export type EvidenceView = 'hotspots' | 'callTree' | 'flame' | 'butterfly' | 'timeline' | 'diff';

/** Open `view` on `profileId` with this selection. */
export interface Evidence {
  view: EvidenceView;
  profileId: number;
  baselineId: number | null;
  frame: FrameDto | null;
  /** Top-down path to expand. */
  path: number[];
  metric: MetricId;
  range: ProfileTimeRange | null;
  threads: number[] | null;
  eventKind: ProfileEventKind | null;
}

export interface ProfileFinding {
  kind: ProfileFindingKind;
  severity: FindingSeverity;
  title: string;
  detail: string;
  valuePct: number | null;
  evidence: Evidence;
}

/** `profiling_moment({ request })` → ProfileMoment[]: profile samples in a log window. */
export interface ProfileMomentRequest {
  fromMs: number;
  toMs: number;
  /** Top functions per profile, default 5, max 50. */
  top?: number | null;
}

export interface ProfileMoment {
  profileId: number;
  name: string;
  /** Pass as `view.range` to open the window in the profile views. */
  range: ProfileTimeRange;
  samples: number;
  metric: MetricId;
  measurement: Measurement;
  value: number;
  top: HotspotRow[];
}

export type HeapSort = 'name' | 'count' | 'bytes' | 'retained';

/** `profiling_heap_types({ request })` → HeapTypesPage. `retained` needs the graph. */
export interface HeapTypesRequest {
  snapshotId: number;
  sort?: HeapSort;
  search?: string | null;
  offset?: number;
  limit?: number;
}

export interface HeapTypeRow {
  name: string;
  count: number;
  bytes: number;
  retained: number | null;
  bytesPct: number;
}

export interface HeapTypesPage {
  rows: HeapTypeRow[];
  totalRows: number;
  totalBytes: number;
  totalCount: number;
  hasGraph: boolean;
}

export type HeapDiffSort = 'bytesDelta' | 'countDelta' | 'name';

/** `profiling_heap_diff({ request })` → HeapDiffPage (default: bytes growth first). */
export interface HeapDiffRequest {
  baselineId: number;
  targetId: number;
  search?: string | null;
  sort?: HeapDiffSort;
  offset?: number;
  limit?: number;
}

export interface HeapDiffRow {
  name: string;
  countBefore: number;
  countAfter: number;
  countDelta: number;
  bytesBefore: number;
  bytesAfter: number;
  bytesDelta: number;
  isNew: boolean;
  isGone: boolean;
}

export interface HeapDiffPage {
  rows: HeapDiffRow[];
  totalRows: number;
  bytesBefore: number;
  bytesAfter: number;
  countBefore: number;
  countAfter: number;
}

/** `profiling_heap_growth({ request })` → HeapGrowthPage (≥ 3 snapshots, oldest first). */
export interface HeapGrowthRequest {
  snapshotIds: number[];
  search?: string | null;
  steadyOnly?: boolean;
  offset?: number;
  limit?: number;
}

export interface HeapGrowthRow {
  name: string;
  bytes: number[];
  counts: number[];
  slopeBytes: number;
  growthBytes: number;
  /** Never shrank and grew overall: a leak candidate. */
  steady: boolean;
}

export interface HeapGrowthPage {
  /** Steady first, then by slope. */
  rows: HeapGrowthRow[];
  totalRows: number;
  steadyCount: number;
}

/** `profiling_heap_dominators({ request })` → HeapObjectsPage (children of `parent`, null = top). */
export interface HeapDominatorsRequest {
  snapshotId: number;
  parent?: number | null;
  offset?: number;
  limit?: number;
}

/** `profiling_heap_instances({ request })` → HeapObjectsPage (objects of a type by retained size). */
export interface HeapInstancesRequest {
  snapshotId: number;
  typeName: string;
  offset?: number;
  limit?: number;
}

export interface HeapObjectRow {
  object: number;
  typeName: string;
  shallow: number;
  retained: number;
  childCount: number;
}

export interface HeapObjectsPage {
  rows: HeapObjectRow[];
  totalRows: number;
  totalBytes: number;
}

/** `profiling_heap_findings({ request })` → HeapFinding[] (Rust `HeapFindingsRequest`). */
export interface HeapFindingsRequest {
  snapshotId: number;
  /** The snapshot before it, for growth. */
  baselineId?: number | null;
}

export type HeapFindingKind = 'topRetainers' | 'largeObjects' | 'finalizable' | 'staticRetention' | 'eventHandlers' | 'strings' | 'growth';
/** The memory view that shows a heap finding's evidence. */
export type HeapFindingView = 'types' | 'dominators' | 'paths' | 'diff';

export interface HeapFinding {
  kind: HeapFindingKind;
  severity: FindingSeverity;
  title: string;
  detail: string;
  valueBytes: number | null;
  typeName: string | null;
  view: HeapFindingView;
}

/** `profiling_heap_paths({ request })` → HeapPaths ("why is this alive"). */
export interface HeapPathsRequest {
  snapshotId: number;
  object: number;
  /** Default 5, max 20. */
  maxPaths?: number | null;
  /** Default and max 256. */
  maxDepth?: number | null;
}

export interface PathStep {
  object: number;
  typeName: string;
  /** Reference from the previous step (field, `[index]`); null on the first. */
  edge: string | null;
}

export interface HeapPaths {
  /** Root-referenced object first, requested object last; shortest first. */
  paths: PathStep[][];
  unreachable: boolean;
  truncated: boolean;
}
// ---------------------------------------------------------------------------------------------
// Explain and share: redaction, dashboards, reports, images, bundles
// (crates/imt-core/src/{redact,dashboard,report,bundle}, app/src-tauri/src/share.rs)
//   overview           { request: { query } }    -> Analyzed<Finding[]>   (the health check)
//   set_redacted_view  { enabled }               -> WorkspaceSummary   (while on, pages, facets, source context, query
//                                                  tables and dashboard tiles come back redacted; `invalid` when the
//                                                  profile has no [redact] rules)
//   preview_redaction  { request }               -> RedactionPreview
//   redaction_summary  { request }               -> RedactionSummary
//   dashboard_tile     { request: TileRequest }  -> TileData
//   export_report      { request }               -> number (the export job)
//   save_visual        { request }               -> VisualSaved
//   open_bundle        { path }                  -> BundleOpened   (an old share bundle; sharing is `save_project` now:
//                                                  estimate_bundle and create_bundle always reject with `unsupported`)
// ---------------------------------------------------------------------------------------------

/** What the UI does when the user follows a health-check finding (Rust `FindingAction`). */
export type FindingAction =
  | { kind: 'zoom'; from: number; to: number; recordId: number | null }
  | { kind: 'exception'; fingerprint: string; recordId: number | null }
  | { kind: 'setupFormat'; path: string }
  | { kind: 'openPanel'; panel: string };

/** What a health-check finding is about (Rust `FindingKind`); unknown kinds from a newer engine are shown as they are. */
export type FindingKind =
  | 'crash'
  | 'errorSpike'
  | 'memoryGrowth'
  | 'silence'
  | 'slowOutlier'
  | 'rateChange'
  | 'exception'
  | 'unreadLines'
  | 'noTimestamps'
  | 'fileNotOpened'
  | 'dropped'
  | 'pluginQuarantined'
  | 'notShown';

/** One line of the automatic health check (`overview { request: { query } } -> Finding[]`), most severe first. */
export interface Finding {
  /** Absent from engines older than the UX pass. */
  kind?: FindingKind | string;
  severity: Severity;
  title: string;
  detail: string;
  action: FindingAction | null;
}

/** What the Home dashboard asks for (`home_summary { request }`, Rust `home::HomeRequest`). */
export interface HomeRequest {
  /** Usually no filters, just the grouping. */
  query: Query;
  /** History buckets, clamped by the engine to 10..=240. */
  buckets: number;
  /** Run the analyses (anomalies, exception groups)? They are the slow part: ask without them first, then with. */
  analyses?: boolean;
}

/** Markers of one kind in one history bucket. */
export interface HomeMarker {
  kind: string;
  /** The first marker's time (Unix ms). */
  ts: number;
  count: number;
}

/** One anomaly to mark on the Home history. */
export interface HomeAnomaly {
  kind: AnomalyKind;
  from: number;
  to: number;
  message: string;
}

/** The Home dashboard's numbers (Rust `home::HomeSummary`). A query: no side effects. */
export interface HomeSummary {
  sources: number;
  events: number;
  /** Error and Fatal events. */
  errors: number;
  warnings: number;
  /** Time span of the timed events (Unix ms); null without timestamps. */
  from: number | null;
  to: number | null;
  /** Width of one history bucket in ms (0 without timestamps). */
  bucketMs: number;
  /** `[bucket][level index in LEVELS order]` = events (empty without timestamps). */
  histogram: number[][];
  /** Error and Fatal events per bucket. */
  errorTrend: number[];
  /** Warning events per bucket. */
  warningTrend: number[];
  /** Markers per kind and bucket, by time (at most 200). */
  markers: HomeMarker[];
  /** Every anomaly found. */
  anomalyCount: number;
  /** The anomalies to mark, by start time (at most 200). */
  anomalies: HomeAnomaly[];
  exceptionGroups: number;
  /** Events in all exception groups. */
  exceptionEvents: number;
  /** Whether the analyses ran; when false the anomaly and exception numbers are zero because they were not asked for. */
  analysed: boolean;
  /** How much of the data the analyses covered. */
  coverage: Coverage;
}

/** Built-in redaction detectors, in the order the engine applies them (`[redact] detectors`). */
export type RedactDetector =
  | 'bearer-token'
  | 'jwt'
  | 'api-key'
  | 'connection-string'
  | 'password'
  | 'user-path'
  | 'email'
  | 'credit-card'
  | 'ipv4'
  | 'ipv6'
  | 'guid'
  | 'phone'
  | 'hostname';
export const REDACT_DETECTORS: readonly RedactDetector[] = [
  'bearer-token',
  'jwt',
  'api-key',
  'connection-string',
  'password',
  'user-path',
  'email',
  'credit-card',
  'ipv4',
  'ipv6',
  'guid',
  'phone',
  'hostname',
];

/** mask: `<email>` · hash: `<email:1a2b3c4d5e>` (salted per export) · drop: removed. */
export type RedactMode = 'mask' | 'hash' | 'drop';

/** Matches of one rule (detector name or custom rule name). */
export interface RuleHits {
  rule: string;
  count: number;
}

export interface RedactionPreviewRequest {
  sample: string;
  /** A candidate (unsaved) profile; absent = the active profile. */
  toml?: string | null;
}

export interface RedactionPreview {
  before: string;
  after: string;
  /** Every active rule in order, zero counts included. */
  hits: RuleHits[];
  /** Why the `[redact]` section does not compile. */
  error: string | null;
}

export interface RedactionSummaryRequest {
  query: Query;
  samples?: number;
}

export interface RedactionSample {
  recordId: number;
  before: string;
  after: string;
}

/** What redaction would change in the events of a query (shown before an export, report or bundle). */
export interface RedactionSummary {
  /** Events looked at (at most 200,000). */
  scanned: number;
  /** Events with at least one match. */
  changed: number;
  truncated: boolean;
  hits: RuleHits[];
  samples: RedactionSample[];
}

/** What a dashboard tile shows (`[[dashboard.tile]] type`). */
export type TileKind = 'chart' | 'stat' | 'table' | 'top' | 'alerts' | 'annotations';
export const TILE_KINDS: readonly TileKind[] = ['chart', 'stat', 'table', 'top', 'alerts', 'annotations'];

/** `[[dashboard.tile]]` (keys are the same in the profile TOML and on the wire). Position and size are in grid units. */
export interface TileCfg {
  id: string;
  title: string;
  type: TileKind;
  x: number;
  y: number;
  w: number;
  h: number;
  /** stat/table: a query-language program; chart/top/alerts: a filter. */
  query?: string;
  /** chart: id of a profile `[[chart]]`. */
  chart?: string;
  /** chart: inline aggregate (`count()`, `avg(cpu)`) when there is no `chart`. */
  value?: string;
  /** chart: inline split fields. */
  by?: string[];
  style?: 'line' | 'bar' | 'stacked';
  /** top: the field. */
  field?: string;
  limit?: number;
  /** stat: unit shown next to the value. */
  unit?: string;
  /** A sliding window of its own ("15m"); absent = the window's query. */
  window?: string;
}

/** `[[dashboard]]`. */
export interface DashboardCfg {
  id: string;
  title: string;
  /** Grid columns (1–24). */
  columns: number;
  tile: TileCfg[];
}

export interface TileRequest {
  tile: TileCfg;
  /** The window's query (filters, time range or live window). */
  query: Query;
}

/** One sparkline point: bucket start and value (null when the bucket is empty). */
export type SparkPoint = [number, number | null];

export type TileData =
  | { kind: 'chart'; data: ChartData }
  | { kind: 'stat'; value: number | null; text: string; label: string; unit: string | null; sparkline: SparkPoint[] }
  | { kind: 'table'; columns: string[]; rows: unknown[][]; truncated: boolean }
  | { kind: 'top'; facet: Facet }
  | { kind: 'alerts'; items: Anomaly[] }
  | { kind: 'annotations'; items: AnnotationDto[] };

/** An annotation copied into a report. */
export interface ReportAnnotation {
  from: number;
  to?: number | null;
  label: string;
  kind?: string;
}

/** A report block; `id` is stable within the report. Unknown types (a newer IMT's) are kept as they are. */
export type ReportBlock =
  | { id: string; type: 'text'; markdown: string }
  | { id: string; type: 'events'; caption?: string; ids: number[] }
  | { id: string; type: 'chart'; caption?: string; title?: string; spec?: ChartDef | null; svg?: string | null }
  | { id: string; type: 'timeline'; caption?: string; svg?: string | null; from?: number | null; to?: number | null }
  | { id: string; type: 'table'; caption?: string; query?: string; columns: string[]; rows: unknown[][] }
  | { id: string; type: 'finding'; severity: string; title: string; detail: string }
  | { id: string; type: 'annotations'; caption?: string; items: ReportAnnotation[] };

export type ReportBlockType = ReportBlock['type'];

export interface ReportSection {
  id: string;
  title: string;
  blocks: ReportBlock[];
}

/** An incident report (crates/imt-core/src/report), saved in the project. */
export interface Report {
  version: number;
  id: string;
  title: string;
  /** Markdown subset: headings, lists, quotes, code, **strong**, *em*, `code`, [links](https://…). */
  summary: string;
  createdAt: number;
  updatedAt: number;
  sections: ReportSection[];
}

export type ReportFormat = 'html' | 'pdf';
export type ExportTheme = 'light' | 'dark';

export interface ReportExportRequest {
  report: Report;
  format: ReportFormat;
  path: string;
  redact: boolean;
  theme: ExportTheme;
  /** Replace an existing file (see `ExportRequest.overwrite`). */
  overwrite?: boolean;
}

/** A chart or timeline image to save: a `data:image/png;base64,` URL, or SVG markup (sanitised by the engine). */
export interface VisualRequest {
  path: string;
  kind: 'png' | 'svg';
  data: string;
  /** Replace an existing file (see `ExportRequest.overwrite`). */
  overwrite?: boolean;
}

export interface VisualSaved {
  path: string;
  bytes: number;
}

/** Raw files as they are, or the events matching a query (as CLEF). */
export type BundleContent = 'raw' | 'events';

export interface SkippedSource {
  path: string;
  reason: string;
}

export interface BundleEntry {
  path: string;
  kind: 'project' | 'profile' | 'log' | 'events' | 'report' | 'dashboards';
  bytes: number;
  sha256: string;
}

export interface BundleManifest {
  format: string;
  schema: number;
  name: string;
  createdAt: number;
  imtVersion: string;
  content: BundleContent;
  redacted: boolean;
  redaction: Record<string, number>;
  events: number | null;
  entries: BundleEntry[];
  skipped: SkippedSource[];
}

/** `open_bundle`: like `open_project`, read-only, with the bundle's manifest (null for other `.imtz` files). */
export interface BundleOpened {
  path: string;
  name: string;
  summary: WorkspaceSummary;
  profile?: ProfileDoc;
  ui: unknown;
  reports: Report[];
  missing: string[];
  readOnly: true;
  bundle: BundleManifest | null;
  /** Where the bundle was extracted. */
  folder: string;
}

// ---------------------------------------------------------------------------------------------
// Ease of use: import progress, profile history, the demo, overwrite protection
// (crates/imt-core/src/workspace, app/src-tauri/src/{workspace_commands,profile}.rs)
//   import_progress    {}                         -> ImportProgress   (readable while an import runs)
//   profile_history    {}                         -> ProfileVersionInfo[]  (newest first, at most 20)
//   profile_version    { file }                   -> ProfileDoc       (not applied until saved again)
//   demo_paths         {}                         -> string[]         (the bundled sample folders; empty when not installed)
//
// Every command that writes a file the user chose takes `overwrite` (default false): export_events
// (`request.overwrite`), export_profile, save_project, export_report (`request.overwrite`), save_visual
// (`request.overwrite`), export_config_diff, export_settings, export_diagnostics. Without it an
// existing file is never replaced: the command fails with `"<path> already exists; confirm to replace it"`.
// ---------------------------------------------------------------------------------------------

/** Suffix of the error a write command returns instead of replacing an existing file. */
export const FILE_EXISTS_SUFFIX = ' already exists; confirm to replace it';

/** The path a `FILE_EXISTS` error refers to, or null for any other error. */
export function fileExistsPath(error: unknown): string | null {
  const text = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  return text.endsWith(FILE_EXISTS_SUFFIX) ? text.slice(0, -FILE_EXISTS_SUFFIX.length) : null;
}

/** Progress of the running (or last) import. */
export interface ImportProgress {
  running: boolean;
  /** Files read so far, of `total`. */
  done: number;
  total: number;
  /** The user cancelled the import: the files not read yet are listed as unrecognised ("not imported"). */
  cancelled: boolean;
  bytesDone: number;
  bytesTotal: number;
  /** Path of a file being read now; null between files and after the import. */
  current: string | null;
  elapsedMs: number;
  /** Estimated time left from the bytes read so far; null until it can be estimated. */
  etaMs: number | null;
}

/** A saved previous version of the active profile. */
export interface ProfileVersionInfo {
  /** Opaque file name to pass to `profile_version`. */
  file: string;
  /** The profile's `name` in that version ("(unreadable)" when it does not parse). */
  name: string;
  /** Stamp of the save, `YYYYMMDD-HHMMSSmmm` (UTC). */
  savedAt: string;
  /** The same instant in epoch ms; null when the stamp cannot be read. */
  savedAtMs: number | null;
}

// ---- Cancellable queries -------------------------------------------------------------------

/**
 * Query-language runs with progress, as analysis jobs. Mirrors `app/src-tauri/src/query_cancel.rs` and
 * `crates/imt-core/src/ql/control.rs`. `onProgress` is a Tauri `Channel<QueryStreamEvent>`: first the run's job
 * (cancel it with `jobs_cancel`), then progress sampled every ~150 ms.
 *
 *   run_query_stream { request: { query }, onProgress } -> StreamedQueryOutcome
 */

/** What a running query is doing: `done` of `total` records of the current `stage`. */
export interface QueryProgress {
  stage: 'matching' | 'evaluating';
  done: number;
  total: number;
}

/** What a run's channel carries. Mirrors `query_cancel::QueryStreamEvent`. */
export type QueryStreamEvent = { kind: 'started'; jobId: number } | ({ kind: 'progress' } & QueryProgress);

/** Like `QueryOutcome`, or `{ cancelled: true }` after its job was cancelled (a normal outcome, not an error). */
export type StreamedQueryOutcome = QueryOutcome | { cancelled: true };

// What a native dialog asks for (`pick_paths`, see core/native-dialog.ts): generated from `paths::PickRequest`.
export type { PickFilter, PickKind, PickRequest } from './generated/paths';

/** What went wrong while reading a source (Rust `imt_plugin_api::problems::ProblemKind`): the same kinds as `SourceProblemKind`. */
export type ProblemKind = SourceProblemKind;

/** An engine-proposed fix of an import problem (Rust `ports::DiagnosticFix`). */
export interface DiagnosticFix {
  title: string;
  /** A `[format]` fragment merged over the parser of the affected sources. */
  parserOverride: string;
  timestamp?: TimestampSuggestion;
}

/** One kind of import problem in one source, or a file that was not imported (`import_problems`). */
export interface DiagnosticRow {
  source: number | null;
  sourceName: string;
  path: string;
  kind: ProblemKind;
  severity: Severity;
  count: number;
  /** At most 20 1-based line numbers of the first occurrences. */
  sampleLines: number[];
  detail?: string;
  fix?: DiagnosticFix;
}

/** A proposed parser override for some sources (`evaluate_fix` argument `fix`): a `[format]` fragment merged over their parser. */
export interface QuickFix {
  parserOverride: string;
  sources?: number[];
}

/** How a quick fix does on the stored events it applies to: "fixes `fixed` of `checked`". */
export interface FixEvaluation {
  fixed: number;
  checked: number;
  unchecked: number;
  notes?: string[];
}

/** Progress of the full-text index (`index_status`). */
export interface IndexStatus {
  indexed: number;
  total: number;
  ready: boolean;
}

/** How much of the matching data an analysis looked at (Rust `ports::Coverage`); see `isPartial`. */
export interface Coverage {
  /** Events the analysis's query matched. */
  matched: number;
  /** Events it looked at (fewer than `matched` when a limit stopped it). */
  analysed: number;
  /** The limits that bounded the result, in words (shown as they are). */
  limits: string[];
}

/** An analysis result with its coverage (Rust `ports::Analyzed`). */
export interface Analyzed<T> {
  result: T;
  coverage: Coverage;
}

/** Whether an analysis did not look at every matching event, or a limit bounded its result. */
export function isPartial(c: Coverage): boolean {
  return c.analysed < c.matched || c.limits.length > 0;
}

/** Coverage of an analysis that looked at all `n` matching events. */
export function fullCoverage(n: number): Coverage {
  return { matched: n, analysed: n, limits: [] };
}

// ---- .NET profiling (plugin net.decryptology.dotnet, tool `dotnet-profiler`; Rust: crates/imt-plugin-dotnet) ----

/** A process that can be recorded (Rust `ProcessInfo`). */
export interface DotnetProcess {
  pid: number;
  name: string;
  commandLine: string | null;
  runtimeVersion: string | null;
  /** `x86`, `x64`, `arm64` when known. */
  arch: string | null;
  /** .NET Framework processes are recorded with ETW (administrator), .NET Core with EventPipe. */
  runtime: 'framework' | 'core';
  /** Recording it needs administrator rights IMT does not have. */
  needsElevation: boolean;
}

export type DotnetCaptureKind = 'cpu' | 'allocations' | 'heap';

/** `start` (Rust `Request`). */
export interface DotnetCaptureRequest {
  pid: number;
  kind: DotnetCaptureKind;
  /** null = until stopped (a heap snapshot ends by itself). 1..3600. */
  durationSec: number | null;
}

export type DotnetCaptureState = 'recording' | 'stopping' | 'done' | 'failed' | 'cancelled';

/** A capture (Rust `Status`). */
export interface DotnetCaptureStatus {
  id: string;
  pid: number;
  processName: string;
  kind: DotnetCaptureKind;
  state: DotnetCaptureState;
  elapsedMs: number;
  bytes: number;
  /** The NetTrace file once done; open it with `profiling_open`. */
  path: string | null;
  error: string | null;
  /** The error is missing rights: offer "Restart as administrator". */
  needsElevation: boolean;
  /** It stopped at the size limit. */
  truncated: boolean;
  notes: string[];
}

/** A process in an `.etl` trace (Rust `EtlProcess`). */
export interface EtlProcess {
  pid: number;
  name: string;
  commandLine: string | null;
  samples: number;
  /** CLR events (0: not .NET or not recorded). */
  clrEvents: number;
  pointerSize: number;
}

/** `etl-convert` result. */
export interface EtlConverted {
  path: string;
  notes: string[];
}

// ---- Recent files and projects (`recent_list`, `recent_remove`, `recent_clear`; event `imt://recent-changed`) ----
// Generated from `recent.rs`. Rust records an entry after a granted open or save.
export type { RecentKind, RecentLists, RecentProject } from './generated/recent';

// ---- Third-party plugin packages (`plugin_package_*`, see tools-backend.ts `PluginPackagesBackend`) ----
// Generated from `plugin_packages`: a package's review before installing, and what is installed.
export type {
  DamagedPluginDto,
  InstalledPluginDto,
  InstalledPluginsDto,
  InstalledState,
  PackageReviewDto,
  PermissionDto,
  PermissionRequestDto,
  RefusalDto,
  SignatureDto,
  SignatureKind,
  UpdateDto,
} from './generated/plugins';

// ---- The one error of every command (app/src-tauri/src/error.rs; ADR 0016) ----

// What every command rejects with, and its codes: generated from `error::CmdError` and `error::ErrorCode`.
export type { CmdErrorDto, ErrorCode } from './generated/errors';

// ---- Native profiling (plugin net.decryptology.native; tools `native-profiler` and `native-profiler.etw`;
// Rust: crates/imt-plugin-native, ADR 0022) ----

/** A process that can be profiled (Rust `NativeProcess`). */
export interface NativeProcess {
  pid: number;
  name: string;
  path: string | null;
  /** `x86`, `x64`, `arm64` when known. */
  arch: string | null;
  /** The sampler can open it; otherwise only ETW (administrator) records it. */
  accessible: boolean;
}

/** `sampling`: suspend-and-sample, no rights needed; `etw`: ETW in the plugin's elevated process. */
export type NativeCaptureMode = 'sampling' | 'etw';

/** `defaults` of `native-profiler` (the settings' symbol options and the sampling bounds). */
export interface NativeDefaults {
  symbolPath: string;
  /** Downloads from the Microsoft symbol server are allowed (off by default). */
  symbolServer: boolean;
  rateHz: number;
  maxRateHz: number;
  maxDurationSec: number;
  /** False off Windows. */
  supported: boolean;
}

/** `start` of either tool (Rust `request::Raw`): `rateHz` for sampling only, `heap` for ETW only. */
export interface NativeCaptureRequest {
  pid: number;
  /** null/absent = until stopped. 1..3600. */
  durationSec?: number | null;
  /** Samples per second per thread, 1..1000 (sampling). */
  rateHz?: number;
  /** Record heap allocations with their stacks (ETW). */
  heap?: boolean;
  /** Folders with PDBs, separated by `;`. */
  symbolPath?: string;
  symbolServer?: boolean;
}

export type NativeCaptureState = 'recording' | 'stopping' | 'done' | 'failed' | 'cancelled';

/** A capture (Rust `CaptureStatus`). */
export interface NativeCaptureStatus {
  id: string;
  pid: number;
  processName: string;
  mode: NativeCaptureMode;
  state: NativeCaptureState;
  elapsedMs: number;
  bytes: number;
  /** Samples taken so far (sampling). */
  samples: number;
  /** The NetTrace file once done; open it with `profiling_open`. */
  path: string | null;
  error: string | null;
  /** The error is missing rights. */
  needsElevation: boolean;
  truncated: boolean;
  notes: string[];
}

// ---- Dumps and UMDH heap logs (app/src-tauri/src/dumps.rs; ADR 0023) ----
// Generated from `imt_dump::api`. Commands (invoke name -> args -> result):
//   dumps_sniff     { paths: string[] }                    -> DumpSniffResult[]   (first 64 KiB; nothing is opened)
//   dumps_open      { path: string }                       -> DumpDocSummary      (a path the user chose)
//   dumps_list      {}                                     -> DumpDocSummary[]
//   dumps_close     { id: number }                         -> boolean
//   dumps_overview  { id: number }                         -> DumpOverview
//   dumps_threads   { request: ThreadsRequest }            -> ThreadsPage
//   dumps_stack     { request: StackRequest }              -> StackDto
//   dumps_modules   { request: ModulesRequest }            -> ModulesPage
//   dumps_findings  { request: DumpFindingsRequest }       -> DumpFinding[]       (one dump, or UMDH logs of one process)
//   dumps_umdh_diff { request: UmdhDiffRequest }           -> UmdhDiffPage
//   dumps_umdh_stack { request: UmdhStackRequest }         -> UmdhStackDto
//   dumps_umdh_tree { request: UmdhTreeRequest }           -> UmdhTreeNode[]
//   dumps_markers   {}                                     -> DumpMarker[]        (documents with a time, for the timeline)
//   dumps_explain   { request: DumpFindingsRequest }       -> string              (the compact, redacted AI question)
// "Create dump of process…" is the plugin tool `dumps.capture` (`tool_invoke`: `processes`, `create`).
export type {
  DocKind,
  DumpDocSummary,
  DumpFinding,
  DumpFindingKind,
  DumpFindingsRequest,
  DumpHeadline,
  DumpMarker,
  DumpOverview,
  DumpSeverity,
  DumpSymbolProblems,
  DumpSymbolStatus,
  DumpSniffKind,
  DumpSniffResult,
  ExceptionCard,
  FindingEvidence,
  FrameRow,
  FrameTrust,
  HandlesDto,
  HandleTypeDto,
  MemoryDto,
  ModuleRow,
  ModulesPage,
  ModulesRequest,
  StackDto,
  StackRequest,
  ThreadRow,
  ThreadsPage,
  ThreadsRequest,
  ThreadState,
  UmdhDiffPage,
  UmdhDiffRequest,
  UmdhGroup,
  UmdhRow,
  UmdhStackDto,
  UmdhStackRequest,
  UmdhTreeNode,
  UmdhTreeRequest,
} from './generated/dumps';

// ---- Parsers (ADR 0026; app/src-tauri/src/parsers.rs, crates/imt-core/src/parser) ----
// A parser is one TOML file describing one log format; the library holds the built-in parsers, the user's and
// overrides of built-ins. Parsing, validation, matching and drafting are the engine's; the UI sends TOML text and ids.
//   parsers_list        {}                                         -> ParserLibraryDto   (stale = sources of this window read with a
//                                                                     parser that has since changed, been disabled or removed)
//   parser_get          { id }                                     -> ParserDocDto
//   parser_validate     { toml }                                   -> ParserValidationDto   (errors with line/column)
//   parser_save         { toml, previousId?: string|null, overwrite?: boolean } -> ParserLibraryDto
//                       (creates/updates a user parser; saving with a built-in's id creates an override; if a USER parser
//                       with that id exists and previousId !== id and !overwrite it fails with code "fileExists"; retry with overwrite)
//   parser_duplicate    { id }                                     -> ParserDocDto      (a saved copy with a new id, e.g. "<id>-copy")
//   parser_remove       { id }                                     -> ParserLibraryDto  (user parsers and overrides only; removing
//                                                                     an override restores the built-in)
//   parser_set_enabled  { id, enabled }                            -> ParserLibraryDto
//   parser_import_check { path }                                   -> ParserImportCheckDto  (reads + validates a .toml the user
//                                                                     picked; relation vs the library)
//   parser_export       { id, path, overwrite?: boolean }          -> void   (FILE_EXISTS without overwrite)
//   parser_draft        { request: ParserDraftRequest }            -> ParserDocDto   (the format wizard's candidate: a new parser
//                                                                     from a regex or Serilog template, or a copy of `base` with a
//                                                                     timestamp format added; NOT saved)
//   parsers_reread      {}                                         -> number   (job id: re-reads this window's stale sources and
//                                                                     retries unrecognised files)
//   parser_assign       { sourceId, parserId: string | null }      -> number   (job id: "Parse with…" re-reads one source with that
//                                                                     parser; null = automatic choice)
export type {
  BrokenParserDto,
  ParserDocDto,
  ParserDraftRequest,
  ParserEntryDto,
  ParserErrorDto,
  ParserExistingDto,
  ParserImportCheckDto,
  ParserLibraryDto,
  ParserOrigin,
  ParserRelation,
  ParserSourceDto,
  ParserValidationDto,
} from './generated/parsers';
// Notes (ADR 0021): generated from `imt_notes` and `notes::commands`.
export type {
  Note,
  NoteAnchor,
  NoteChange,
  NoteDraft,
  NoteImage,
  NoteImageRef,
  NoteItem,
  NoteRecord,
  NoteRegion,
  NoteSeverity,
  NotesList,
  NotesRestore,
} from './generated/notes';

/*
 * The Store pane and the read-only SQL console (ADR 0030; types generated from `imt_core::ports::{inspect, console}`):
 *   store_overview    {}                                   -> StoreOverview
 *   store_segments    { request: SegmentsRequest }         -> SegmentsPage   (at most 500 rows a page)
 *   store_fields      {}                                   -> StoreFields
 *   store_index       {}                                   -> StoreIndex
 *   store_memory      {}                                   -> StoreMemory    (with the worker process and the window's jobs)
 *   store_maintenance { action: MaintenanceAction }        -> number         (the job's id; the UI confirms first)
 *   sql_schema        {}                                   -> SqlSchema      (views, columns and functions to complete)
 *   sql_run           { sql, explain, onEvent }            -> SqlOutcome     (an analysis job: `onEvent` first carries
 *                                                                              `{ kind: 'started', jobId }`; cancel with `jobs_cancel`)
 *   sql_page          { request: SqlPageRequest }          -> SqlPage
 *   sql_ids           { request: SqlIdsRequest }          -> SqlIds         (the event ids of a kept result)
 *   sql_export        { sql, path, overwrite }             -> number         (an export job's id: all rows as CSV, up to 5 M rows,
 *                                                                              2 GiB and 10 min; cells starting with = + - @ get a leading ')
 * Limits: 10,000 rows kept per run, pages of 500, 30 s, 128 MB, 64 KB of SQL. Refusals and invalid SQL come back as
 * `{ error: QlError }` (offsets in UTF-8 bytes); a limit hit rejects the command with an `invalid` CmdError.
 */
export type {
  FieldStorage,
  LastImport,
  MaintenanceAction,
  MaintenanceReport,
  RunningJob,
  SegmentRow,
  SegmentsPage,
  SegmentsRequest,
  SqlColumn,
  SqlIds,
  SqlIdsRequest,
  SqlPage,
  SqlPageRequest,
  SqlResult,
  SqlSchema,
  SqlView,
  SqlViewColumn,
  StoreActivity,
  StoreField,
  StoreFieldType,
  StoreFields,
  StoreIndex,
  StoreMemory,
  StoreOverview,
  WorkerMemory,
} from './generated/store';
/** A console run's outcome (`sql_run`). Mirrors `store_pane::SqlOutcome`. */
export type SqlOutcome = { result: import('./generated/store').SqlResult } | { error: QlError } | { cancelled: true };

// ---- Known issues, incident summary and playbooks (ADR 0039; crates/imt-core/src/{known_issues,incident,playbook}) ----

/** How a known issue is recognised: a regex over the message and exception, or a query-language filter. */
export type IssueMatcher = { type: 'pattern'; pattern: string } | { type: 'query'; ql: string };

/** A known issue declared in a knowledge file (or suggested by the AI and verified by the engine). */
export interface KnownIssue {
  id: string;
  title: string;
  fix: string;
  matcher: IssueMatcher;
  /** The knowledge file that declares it. */
  file: string;
}

/** Who found the match: the user's declared rule, or an AI suggestion the engine verified. */
export type IssueOrigin = 'rule' | 'ai';

/** A known issue present in the open data, with the evidence. */
export interface IssueMatch {
  issue: KnownIssue;
  origin: IssueOrigin;
  /** The filter that selects the events. */
  query: string;
  count: number;
  /** Up to 5 events that show it. */
  sampleIds: number[];
}

/** `known_issues_match`: what is present, and what the user should fix in their knowledge files. */
export interface IssueReport {
  matches: IssueMatch[];
  checked: number;
  truncated: number;
  warnings: string[];
}

/** `incident_report_build` arguments. */
export interface IncidentRequest {
  title?: string;
  /** Epoch ms. */
  from?: number | null;
  to?: number | null;
  /** Entries per ranking, 1 to 10 (5 when absent). */
  top?: number | null;
}

export type PlaybookExpect = { kind: 'noMatch' } | { kind: 'anyMatch' } | { kind: 'atLeast'; n: number } | { kind: 'atMost'; n: number };

/** What a playbook step does. */
export type PlaybookAction =
  | { type: 'manual' }
  | { type: 'query'; ql: string }
  | { type: 'filter'; ql: string }
  | { type: 'openView'; view: string }
  | { type: 'check'; ql: string; expect: PlaybookExpect };

export interface PlaybookStep {
  id: string;
  title: string;
  instruction: string;
  action: PlaybookAction;
}

/** A saved investigation (a versioned JSON file; see docs/playbooks.md). */
export interface Playbook {
  version: number;
  id: string;
  title: string;
  description: string;
  tags: string[];
  steps: PlaybookStep[];
}

export type PlaybookStatus = 'pass' | 'fail' | 'info' | 'error' | 'skipped';

/** What running a step found; the engine computes it. */
export interface PlaybookStepResult {
  status: PlaybookStatus;
  summary: string;
  count: number | null;
  sampleIds: number[];
  columns: string[];
  rows: unknown[][];
}

export interface PlaybookStepRecord {
  stepId: string;
  status: PlaybookStatus;
  summary: string;
  count: number | null;
  sampleIds: number[];
  note: string;
  at: number;
}

/** One execution of a playbook, with the playbook as it was. Saved in the project. */
export interface PlaybookRun {
  id: string;
  playbook: Playbook;
  startedAt: number;
  steps: PlaybookStepRecord[];
}

/** The runs a project keeps (`PlaybookRecord` v1; the project's UI state key `playbooks`). */
export interface PlaybookRecord {
  version: number;
  runs: PlaybookRun[];
}

export interface PlaybookLibrary {
  playbooks: Playbook[];
  templates: Playbook[];
  warnings: string[];
}

export interface PlaybookRestore {
  record: PlaybookRecord;
  warnings: string[];
}
