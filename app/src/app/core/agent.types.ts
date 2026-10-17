/**
 * Wire types of the scheduled-collection agent (`imt-agent`), mirroring the Rust DTOs in
 * `crates/imt-agent/src/{config,schema,run}.rs` and `app/src-tauri/src/agent_commands.rs`
 * (camelCase JSON). Change both sides together.
 */

// ---- imt-agent.toml (AgentConfigFile) ----

export type TimeZoneChoice = 'local' | 'utc';
export type AgentLogLevel = 'debug' | 'information' | 'warning' | 'error';
export type AgentMetric = 'privateBytes' | 'workingSet' | 'virtualBytes' | 'cpu' | 'handles' | 'threads';

export interface AgentLogSection {
  level: AgentLogLevel;
  maxFileMb: number;
  keepFiles: number;
  maxAgeDays: number;
}

export interface AgentSection {
  outputDir: string;
  stateDir?: string;
  logDir?: string;
  minFreeMb: number;
  tickSecs: number;
  timezone: TimeZoneChoice;
  /** Longest `logman` call of a PerfMon capture, seconds (5-600; the tools plugin's default when missing). */
  logmanTimeoutSecs?: number;
  log: AgentLogSection;
}

/** A missing (or null) limit is off. */
export interface RetentionSection {
  maxRuns?: number | null;
  maxTotalMb?: number | null;
  maxAgeDays?: number | null;
}

export type ProcessTriggerFile = {
  kind: 'process';
  process: string;
  metric: AgentMetric;
  above: number;
  forSecs?: number;
  sampleSecs?: number;
  cooldownSecs?: number;
};
export type LogTriggerFile = { kind: 'log'; path: string; ql: string; pollSecs?: number; cooldownSecs?: number };
export type TriggerFile = ProcessTriggerFile | LogTriggerFile;
export type TriggerKind = TriggerFile['kind'];

export type ActionFile =
  | { kind: 'collect'; planFile?: string; plan?: Record<string, unknown> }
  | { kind: 'monitor'; targets: string[]; duration: string; intervalMs?: number; includeChildren?: boolean }
  | { kind: 'copyLogs'; from: string; include?: string[]; maxAgeDays?: number; maxTotalMb?: number }
  | { kind: 'perfmon'; collector: string; duration: string };
export type ActionKind = ActionFile['kind'];

/** Exactly one of `cron`, `every` and `trigger` (the engine checks). */
export interface ScheduleFile {
  name: string;
  enabled: boolean;
  cron?: string;
  every?: string;
  trigger?: TriggerFile;
  actions: ActionFile[];
}

export interface AgentConfigFile {
  version: number;
  agent: AgentSection;
  retention: RetentionSection;
  schedules: ScheduleFile[];
}

/** One problem at a key path such as `schedules[0].cron` (empty path: the whole file). */
export interface AgentIssue {
  path: string;
  message: string;
}

export interface AgentConfigDoc {
  path: string;
  /** False: `config` is a starter document that is not saved yet. */
  exists: boolean;
  /** Null when the file exists but cannot be read (see `issues`). */
  config: AgentConfigFile | null;
  issues: AgentIssue[];
}

// ---- schema for the editor ----

export type AgentFieldKind = 'text' | 'number' | 'boolean' | 'duration' | 'cron' | 'select' | 'path' | 'list' | 'query' | 'json';

export interface AgentFieldSchema {
  key: string;
  label: string;
  kind: AgentFieldKind;
  required: boolean;
  help: string;
  default: unknown;
  min?: number;
  max?: number;
  options?: { value: string; label: string }[];
}

export interface AgentVariantSchema {
  kind: string;
  label: string;
  help: string;
  fields: AgentFieldSchema[];
}

export interface AgentSchema {
  version: number;
  agent: AgentFieldSchema[];
  log: AgentFieldSchema[];
  retention: AgentFieldSchema[];
  schedule: AgentFieldSchema[];
  /** `cron`, `every` or `trigger`. */
  timing: AgentVariantSchema[];
  triggers: AgentVariantSchema[];
  actions: AgentVariantSchema[];
}

// ---- runs ----

export type RunStatus = 'running' | 'ok' | 'partial' | 'failed' | 'stopped' | 'incomplete';
export type ActionStatus = 'ok' | 'failed' | 'skipped' | 'stopped';

export interface RunSummary {
  id: string;
  dir: string;
  schedule: string;
  status: RunStatus;
  /** "scheduled (…)", "triggered: …" or "manual". */
  reason: string;
  startedMs: number;
  durationMs: number;
  totalBytes: number;
  actions: number;
  error: string | null;
}

export interface RunsListing {
  outputDir: string;
  runs: RunSummary[];
  /** Runs left out beyond the listing limit. */
  truncated: number;
}

export type RunReason = { kind: 'schedule'; timer: string } | { kind: 'trigger'; detail: string } | { kind: 'manual' };

export interface ActionRecord {
  kind: string;
  status: ActionStatus;
  message?: string | null;
  /** Relative to the run folder. */
  outputs: string[];
  bytes: number;
  ms: number;
}

export interface RunManifest {
  schema: number;
  id: string;
  schedule: string;
  reason: RunReason;
  host: string;
  agentVersion: string;
  startedAt: string;
  startedMs: number;
  finishedAt: string | null;
  durationMs: number;
  status: RunStatus;
  actions: ActionRecord[];
  totalBytes: number;
  errors: string[];
}
