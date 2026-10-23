// Wire contract of Settings ▸ Performance (ADR 0031; camelCase JSON), mirrored by
// app/src-tauri/src/performance/{commands,suggest,watch}.rs. Change both sides together.
// Commands: `performance_suggest`, `performance_preset`, `performance_memory`, `performance_set_plugin_memory`
// (restarting a stopped store worker is `health_repair`, see health/). The section's values are applied with `set_settings` (section `performance`).

import { SettingValue } from '../settings.types';

/** Section id of Settings ▸ Performance (Rust `builtin_sections::PERFORMANCE_SECTION`). */
export const PERFORMANCE_SECTION = 'performance';

/** One of Debrief's capped processes. */
export type ProcessRef = { kind: 'storeWorker' } | { kind: 'ai' } | { kind: 'plugin'; pluginId: string };

export type Preset = 'standard' | 'lowMemory';

/** What Suggest settings is computed from (sizes in MB). */
export interface Environment {
  totalMemoryMb: number;
  freeMemoryMb: number;
  cores: number;
  diskFreeMb: number | null;
  diskKind: 'ssd' | 'hdd' | null;
  aiModelMb: number;
}

/** One changed value of a proposal. */
export interface ProposalRow {
  key: string;
  label: string;
  current: SettingValue;
  suggested: SettingValue;
  unit: string | null;
}

/** Suggested (or preset) values next to the current ones; `values` is applied with `set_settings`. */
export interface Proposal {
  environment: Environment | null;
  rows: ProposalRow[];
  values: Record<string, SettingValue>;
}

export type MemoryState = 'warned' | 'stopped' | 'ended';

/** One capped process: the cap of its next start and the memory watcher's last report (MB). */
export interface ProcessMemory {
  process: ProcessRef;
  name: string | null;
  capMb: number;
  /** A plugin's own cap set in the Plugins panel. */
  overrideMb: number | null;
  usedMb: number | null;
  peakMb: number | null;
  state: MemoryState | null;
}

/** Smallest and largest per-plugin cap (Rust `section::{MIN,MAX}_PLUGIN_MB`; the engine re-validates). */
export const MIN_PLUGIN_MB = 64;
export const MAX_PLUGIN_MB = 4096;
