import { CollectorTemplate, CounterPreset, LogFormat } from '../../core/tools.types';

export const DEFAULT_OUTPUT_DIR = '%systemdrive%\\PerfLogs\\IMT\\';

/** What the wizard edits. Counters are a set of paths; the template splits them into presets and extras. */
export interface PerfmonDraft {
  name: string;
  processes: string[];
  counters: string[];
  extraCounters: string[];
  intervalSecs: number;
  format: LogFormat;
  maxSizeMb: number | null;
  durationMinutes: number | null;
  outputDir: string;
}

export const DEFAULT_DRAFT: PerfmonDraft = {
  name: 'IMT',
  processes: [],
  counters: [],
  extraCounters: [],
  intervalSecs: 15,
  format: 'csv',
  maxSizeMb: null,
  durationMinutes: null,
  outputDir: DEFAULT_OUTPUT_DIR,
};

/** Instance name as PerfMon shows it: trimmed, without `.exe`. */
export function normaliseProcess(input: string): string {
  const t = input.trim();
  return /\.exe$/i.test(t) ? t.slice(0, -4) : t;
}

/** Adds a process chip (deduplicated, case-insensitive). */
export function addProcess(list: readonly string[], input: string): string[] {
  const name = normaliseProcess(input);
  if (!name || list.some((p) => p.toLowerCase() === name.toLowerCase())) return [...list];
  return [...list, name];
}

/** Preset ids whose counters are all selected. */
export function fullPresets(presets: readonly CounterPreset[], selected: readonly string[]): string[] {
  return presets.filter((p) => p.counters.length > 0 && p.counters.every((c) => selected.includes(c.path))).map((p) => p.id);
}

/** Selection state of a preset: how many of its counters are checked. */
export function presetState(preset: CounterPreset, selected: readonly string[]): 'all' | 'some' | 'none' {
  const n = preset.counters.filter((c) => selected.includes(c.path)).length;
  if (n === 0) return 'none';
  return n === preset.counters.length ? 'all' : 'some';
}

export function toggleCounter(selected: readonly string[], path: string): string[] {
  return selected.includes(path) ? selected.filter((p) => p !== path) : [...selected, path];
}

export function togglePreset(preset: CounterPreset, selected: readonly string[]): string[] {
  const paths = preset.counters.map((c) => c.path);
  if (presetState(preset, selected) === 'all') return selected.filter((p) => !paths.includes(p));
  return [...selected, ...paths.filter((p) => !selected.includes(p))];
}

/** Draft problems found early for convenience; the engine validates again. */
export function validateDraft(d: PerfmonDraft): string[] {
  const problems: string[] = [];
  if (!d.name.trim() || /[\\/:*?"<>|]/.test(d.name)) problems.push($localize`:@@tools.perfmon.problem.name:Enter a collector name without \\ / : * ? " < > |.`);
  if (!(d.intervalSecs >= 1 && d.intervalSecs <= 86_400)) problems.push($localize`:@@tools.perfmon.problem.interval:The interval must be between 1 and 86400 seconds.`);
  if (d.maxSizeMb != null && d.format !== 'blg') problems.push($localize`:@@tools.perfmon.problem.maxSize:A maximum size (circular log) needs the BLG format.`);
  if (!d.outputDir.trim()) problems.push($localize`:@@tools.perfmon.problem.outputDir:Choose an output folder.`);
  if (!d.counters.length && !d.extraCounters.length) problems.push($localize`:@@tools.perfmon.problem.noCounter:Select at least one counter.`);
  return problems;
}

/** Builds the engine's template. Whole presets travel by id, other checked counters as extras. */
export function toTemplate(d: PerfmonDraft, presets: readonly CounterPreset[]): CollectorTemplate {
  const ids = fullPresets(presets, d.counters);
  const covered = new Set(presets.filter((p) => ids.includes(p.id)).flatMap((p) => p.counters.map((c) => c.path)));
  const extras = [...d.counters.filter((c) => !covered.has(c)), ...d.extraCounters];
  return {
    name: d.name.trim(),
    processes: d.processes,
    presets: ids,
    extraCounters: [...new Set(extras)],
    sampleIntervalSecs: d.intervalSecs,
    format: d.format,
    outputDir: d.outputDir.trim(),
    maxSizeMb: d.format === 'blg' ? d.maxSizeMb : null,
    durationSecs: d.durationMinutes != null ? d.durationMinutes * 60 : null,
  };
}
