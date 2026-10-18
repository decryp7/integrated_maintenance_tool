import { CpuMode, MonitorConfig, MonitorTarget } from '../../core/tools.types';

export const MIN_INTERVAL_MS = 250;

export type TargetKind = 'name' | 'path' | 'pid';

/** A row of the targets editor. */
export interface TargetRow {
  kind: TargetKind;
  value: string;
}

export interface MonitorDraft {
  targets: TargetRow[];
  intervalMs: number;
  cpu: CpuMode;
  includeChildren: boolean;
  /** Opt-in .NET runtime counters (.NET Framework processes). */
  dotnet: boolean;
  /** One counter path per line (Windows). */
  counters: string;
  dir: string;
}

export const DEFAULT_MONITOR_DRAFT: MonitorDraft = {
  targets: [{ kind: 'name', value: '' }],
  intervalMs: 2000,
  cpu: 'machine',
  includeChildren: false,
  dotnet: false,
  counters: '',
  dir: '',
};

export function rowToTarget(row: TargetRow): MonitorTarget | null {
  const v = row.value.trim();
  if (!v) return null;
  if (row.kind === 'pid') {
    const pid = Number(v);
    return Number.isInteger(pid) && pid >= 0 ? { pid } : null;
  }
  return row.kind === 'name' ? { name: v } : { path: v };
}

/** Rows that cannot be used, for an inline message. Empty rows are ignored, not errors. */
export function invalidRows(d: MonitorDraft): string[] {
  return d.targets.filter((r) => r.value.trim() && !rowToTarget(r)).map((r) => $localize`:@@tools.monitor.problem.badPid:"${r.value.trim()}:value:" is not a valid process id.`);
}

export function validateMonitor(d: MonitorDraft): string[] {
  const problems = invalidRows(d);
  if (!d.targets.some((r) => rowToTarget(r))) problems.push($localize`:@@tools.monitor.problem.noTarget:Add at least one process name, path or id to watch.`);
  if (!(d.intervalMs >= MIN_INTERVAL_MS)) problems.push($localize`:@@tools.monitor.problem.interval:The interval must be at least ${MIN_INTERVAL_MS}:min: ms.`);
  return problems;
}

export function toMonitorConfig(d: MonitorDraft): MonitorConfig {
  return {
    targets: d.targets.map(rowToTarget).filter((t): t is MonitorTarget => t !== null),
    intervalMs: d.intervalMs,
    cpu: d.cpu,
    counters: d.counters
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter(Boolean),
    includeChildren: d.includeChildren,
    // Optional on the wire: written only when switched on, so existing configs (and schedules) stay as they were.
    ...(d.dotnet ? { dotnet: true } : {}),
  };
}
