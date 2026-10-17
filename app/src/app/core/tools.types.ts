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
 *   save_project      { path, name, ui }          -> { path, name }
 *   open_project      { path }                    -> ProjectOpened
 *   set_live          { enabled }                 -> number (watched paths)
 *   take_pending_open {}                          -> string[]
 * Events: "imt://open" (take_pending_open has paths), "imt://workspace-changed" (live tail refreshed).
 */
import { Level, WorkspaceSummary } from './api.types';

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
}

export interface MonitorStatus {
  running: boolean;
  dir: string | null;
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
