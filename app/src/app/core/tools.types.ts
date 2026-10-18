/**
 * Wire contract for the tooling, lifecycle and project commands (camelCase JSON). Mirrors
 * app/src-tauri/src/{tools,lifecycle,system,lib}.rs, imt-plugin-windows `perfmon_*`,
 * imt-collect `plan`/`manifest`, imt-monitor `config` and imt-core `project`/`plugin`.
 *
 * Commands (invoke name -> args -> result):
 *   perfmon_presets   {}                          -> CounterPreset[]
 *   perfmon_generate  { template, xmlPath? }      -> GeneratedCollector
 *   perfmon_install   { template }                -> CollectorStatus
 *   perfmon_control   { name, action }            -> CollectorStatus | null
 *   perfmon_outputs   { name }                    -> string[]
 *   monitor_start     { config, dir? }            -> string (folder)
 *   monitor_stop      {}                          -> string | null
 *   monitor_status    {}                          -> MonitorStatus
 *   collect_default_plan {}                       -> string (JSON of CollectPlan)
 *   collect_diagnostics { plan, outDir }          -> CollectResult
 *   elevation         {}                          -> ElevationInfo
 *   restart_elevated  { reopen }                  -> void
 *   app_status        {}                          -> AppStatus
 *   restart_app       { safeMode, reopen }        -> void
 *   plugin_reset      { id }                      -> void
 *   plugin_set_enabled{ id, enabled }             -> boolean (restart needed)
 *   save_session      { ui }                      -> void
 *   last_session      {}                          -> string | null
 *   save_project      { path, name, ui, reports? } -> { path, name }
 *   open_project      { path }                    -> ProjectOpened
 *   set_live          { enabled }                 -> number (watched paths)
 *   take_pending_open {}                          -> string[]
 * Event Log follower (`tool_invoke { tool: 'event-log-follower', op, args }`, imt-plugin-tools `eventlog_tool`):
 *   channels {}                                   -> EventLogChannel[]
 *   validate EventLogQuery                        -> EventLogValidation
 *   start    EventLogStartArgs                    -> EventLogStarted
 *   stop     {}                                   -> string | null (folder)
 *   pause / resume / status {}                    -> EventLogStatus
 * Events: "imt://open" (take_pending_open has paths), "imt://workspace-changed" (live tail refreshed).
 */
import { Level, Report, WorkspaceSummary } from './api.types';

// ---- Performance Monitor -------------------------------------------------------------------

export interface CounterGuidance {
  path: string;
  /** What a problem looks like for this counter; null when unknown. */
  guidance: string | null;
}

export interface CounterPreset {
  id: string;
  name: string;
  description: string;
  counters: CounterGuidance[];
}

export type LogFormat = 'blg' | 'csv' | 'tsv';

/** Input of `perfmon_generate` / `perfmon_install` (`CollectorTemplate` in imt-plugin-windows). */
export interface CollectorTemplate {
  name: string;
  processes: string[];
  presets: string[];
  extraCounters: string[];
  sampleIntervalSecs: number;
  format: LogFormat;
  outputDir: string;
  /** Circular log size in MB (BLG only). */
  maxSizeMb: number | null;
  durationSecs: number | null;
}

export interface GeneratedCollector {
  xml: string;
  path: string;
  /** `logman` command lines to import, start, stop and delete the set. */
  commands: string[];
}

export type CollectorRunState = 'running' | 'stopped' | 'unknown';
export type CollectorAction = 'start' | 'stop' | 'delete' | 'query';

export interface CollectorStatus {
  name: string;
  status: CollectorRunState;
  rootPath: string;
  outputLocation: string;
  segment: boolean;
}

// ---- Process monitor -----------------------------------------------------------------------

export type CpuMode = 'machine' | 'core';

/** One target: exactly one of the fields is set. */
export interface MonitorTarget {
  name?: string;
  path?: string;
  pid?: number;
}

export interface MonitorConfig {
  targets: MonitorTarget[];
  intervalMs: number;
  cpu: CpuMode;
  counters: string[];
  includeChildren: boolean;
  /** Also sample .NET runtime counters (GC, heap, exceptions, contention) of .NET Framework processes; default off. */
  dotnet?: boolean;
}

export interface MonitorStatus {
  running: boolean;
  dir: string | null;
}

// ---- Event Log follower -------------------------------------------------------------------

/** An Event Log level as users pick it (`EventLevel` in imt-plugin-windows). */
export type EventLogLevel = 'critical' | 'error' | 'warning' | 'information' | 'verbose';
export const EVENT_LOG_LEVELS: readonly EventLogLevel[] = ['critical', 'error', 'warning', 'information', 'verbose'];

/** How far back to read before following live events. */
export type EventLogBackfill = 'none' | '1h' | '24h' | '7d';
export const EVENT_LOG_BACKFILLS: readonly EventLogBackfill[] = ['none', '1h', '24h', '7d'];

/** A channel the follower offers (`channels`). */
export interface EventLogChannel {
  name: string;
  /** Reading it needs administrator rights (Security). */
  needsAdmin: boolean;
}

/** What to follow (`validate`, and the base of `start`). Missing levels mean all levels. */
export interface EventLogQuery {
  channels: string[];
  levels?: EventLogLevel[];
  /** A user XPath filter, combined with the levels and the backfill window. */
  xpath?: string;
  backfill?: EventLogBackfill;
}

export interface EventLogChannelCheck {
  name: string;
  ok: boolean;
  error: string | null;
}

/** Result of `validate`: the composed queries and whether each channel accepts them. */
export interface EventLogValidation {
  /** The live query. */
  xpath: string;
  /** The query for the backfill window; null without backfill. */
  backfillXpath: string | null;
  channels: EventLogChannelCheck[];
}

export interface EventLogStartArgs extends EventLogQuery {
  /** Continue after the saved positions (default true). */
  resumeFromBookmark?: boolean;
  /** Output folder; defaults to IMT's data folder. */
  dir?: string;
  /** Size of one rolling file, 1-100 MB (10). */
  maxFileMb?: number;
  /** Rolling files kept, 1-50 (10). */
  maxFiles?: number;
}

export interface EventLogStarted {
  dir: string;
  /** The live query. */
  xpath: string;
  /** Channels that continue from their saved position. */
  resumed: string[];
}

export type EventLogChannelState = 'starting' | 'backfill' | 'live' | 'retrying' | 'stopped';

export interface EventLogChannelStatus {
  channel: string;
  state: EventLogChannelState;
  /** Events written from this channel in this run. */
  events: number;
  /** The last written record (the bookmark). */
  lastRecordId: number | null;
  /** The current failure, while retrying. */
  error: string | null;
}

export interface EventLogLastEvent {
  /** Epoch ms; null when the event had no time. */
  ts: number | null;
  channel: string;
  provider: string;
  eventId: number | string | null;
  level: Level;
}

export interface EventLogErrorEntry {
  /** Epoch ms. */
  at: number;
  channel: string | null;
  message: string;
}

/** `status` while nothing runs. */
export interface EventLogIdle {
  running: false;
  dir: null;
}

/** `status` (and `pause` / `resume`) of a run. */
export interface EventLogRun {
  running: boolean;
  paused: boolean;
  dir: string;
  /** Events written in this run. */
  events: number;
  /** Over the last ten seconds. */
  eventsPerSecond: number;
  lastEvent: EventLogLastEvent | null;
  channels: EventLogChannelStatus[];
  /** Most recent first, at most 20. */
  errors: EventLogErrorEntry[];
  /** Events that could not be read and were skipped. */
  unreadable: number;
  queued: number;
  queueCapacity: number;
  /** The live query. */
  xpath: string;
}

export type EventLogStatus = EventLogIdle | EventLogRun;

/** True when the status describes a run rather than the idle answer. */
export function isEventLogRun(s: EventLogStatus | null | undefined): s is EventLogRun {
  return !!s && 'channels' in s;
}

// ---- Diagnostics collector -----------------------------------------------------------------

export interface LogSpec {
  path: string;
  include: string[];
  maxAgeDays: number | null;
  maxTotalMb: number | null;
}

export interface CustomPattern {
  name: string;
  regex: string;
  replace: string;
}

export interface RedactConfig {
  builtIn: boolean;
  ipAddresses: boolean;
  patterns: CustomPattern[];
}

/** `CollectPlan` of imt-collect (unknown keys are rejected by the engine). */
export interface CollectPlan {
  name: string;
  logs: LogSpec[];
  eventLogs: string[];
  eventLogDays: number;
  msinfo: boolean;
  registry: string[];
  counters: boolean;
  counterPaths: string[];
  environment: boolean;
  envAllowlist: string[];
  macosUnifiedLog: boolean;
  linuxJournal: boolean;
  recentLogHours: number;
  redact: RedactConfig;
  maxBundleMb: number;
  commandTimeoutSecs: number;
}

export interface ManifestItem {
  path: string;
  kind: string;
  origin: string;
  originalBytes: number;
  bytes: number;
  redacted: boolean;
  redactions: number;
  note?: string | null;
}

export interface ManifestSkipped {
  kind: string;
  what: string;
  reason: string;
}

/** The parts of imt-collect's manifest the UI shows. */
export interface Manifest {
  name: string;
  host: string;
  platform: string;
  createdAt: string;
  items: ManifestItem[];
  skipped: ManifestSkipped[];
  /** Redactions per rule name. */
  redaction: Record<string, number>;
  errors: string[];
  totalBytes: number;
  durationMs: number;
}

export interface CollectResult {
  bundle: string;
  manifest: Manifest;
}

// ---- Lifecycle -----------------------------------------------------------------------------

/** The operating system the app runs on (Rust `system::Platform`). */
export type Platform = 'windows' | 'macos' | 'linux';

export interface ElevationInfo {
  elevated: boolean;
  /** True on Windows, where `restart_elevated` works. */
  canRestartElevated: boolean;
  platform: Platform;
}

/** The plugin that contributes the system tools (PerfMon, process monitor, diagnostics, elevation). */
export const TOOLS_PLUGIN = 'net.decryptology.tools';

/** Ids of the tools the UI uses (`tool_invoke { tool }`). */
export const TOOL_IDS = {
  perfmonTemplates: 'perfmon.templates',
  perfmonSets: 'perfmon.sets',
  processMonitor: 'process-monitor',
  eventLog: 'event-log-follower',
  diagnostics: 'diagnostics',
  elevation: 'elevation',
} as const;

/** The shape of a tool operation's arguments (checked by the engine before the tool runs). */
export type ToolSchema =
  | { type: 'any' }
  | { type: 'bool' }
  | { type: 'int'; min: number; max: number }
  | { type: 'text'; maxLen: number }
  | { type: 'enum'; values: string[] }
  | { type: 'list'; items: ToolSchema; maxItems: number }
  | { type: 'object'; fields: ToolField[]; additional: boolean };

export interface ToolField {
  name: string;
  schema: ToolSchema;
  /** Optional fields may be missing or null. */
  required: boolean;
}

export interface ToolOp {
  name: string;
  title: string;
  args: ToolSchema;
  /** Time the operation may take before it is abandoned; null = unlimited. */
  budgetMs: number | null;
}

/** Whether a tool can be used now, and if not, why. */
export type ToolAvailability = { state: 'available' } | { state: 'unsupported'; reason: string } | { state: 'pluginUnavailable'; reason: string };

/** A tool as `tools_list` reports it. */
export interface ToolEntry {
  id: string;
  title: string;
  description: string;
  /** Platforms it runs on; empty = all. */
  platforms: Platform[];
  needsElevation: boolean;
  ops: ToolOp[];
  /** The plugin that contributes it. */
  plugin: string;
  availability: ToolAvailability;
}

export interface AppStatus {
  version: string;
  safeMode: boolean;
  previousCrash: boolean;
  sessionAvailable: boolean;
}

// ---- Projects ------------------------------------------------------------------------------

export interface ProjectSaved {
  path: string;
  name: string;
}

export interface ProjectOpened {
  path: string;
  name: string;
  summary: WorkspaceSummary;
  profile: { name: string; path: string | null; toml: string };
  /** The `ui` value that was passed to `save_project` (see `UiState`). */
  ui: unknown;
  /** Source paths of the project that no longer exist. */
  missing: string[];
  /** Incident reports saved in the project (absent from older shells). */
  reports?: Report[];
}

/**
 * Serializable UI state stored in a project's (and the session's) `ui` field. Every key is owned by
 * a `UiStateContribution`; unknown keys are preserved by the engine and ignored by the UI.
 */
export interface UiState {
  version: number;
  query?: { query: unknown; textInput: string };
  layout?: { id: string; openViews: string[] };
  selection?: { recordId: number | null };
  bookmarks?: Bookmark[];
  savedQueries?: SavedQuery[];
  highlightRules?: HighlightRule[];
  [key: string]: unknown;
}

export interface Bookmark {
  id: string;
  kind: 'record' | 'range';
  note: string;
  /** Record bookmarks. */
  recordId?: number;
  /** A short description captured when bookmarking (message or time range text). */
  label: string;
  ts?: number | null;
  level?: Level;
  /** Range bookmarks (epoch ms). */
  from?: number;
  to?: number;
  createdAt: number;
}

export interface SavedQuery {
  id: string;
  name: string;
  query: unknown;
}

export type HighlightOp = 'eq' | 'ne' | 'contains' | 'notContains' | 'exists' | 'missing' | 'gt' | 'lt';

export interface HighlightRule {
  id: string;
  /** Field name or pseudo-field (`@level`, `@message`, `@source`). */
  field: string;
  op: HighlightOp;
  value: string;
  /** CSS colour used for the row. */
  color: string;
  enabled: boolean;
}
