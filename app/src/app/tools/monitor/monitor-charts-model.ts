import { formatBytes, formatNumber, formatPercent, formatRate } from '../../shared/format/units';
import type { EChartsCoreOption } from 'echarts/core';
import { ChartData, ChartDef, Query, WorkspaceSummary } from '../../core/api.types';
import { EMPTY_QUERY } from '../../core/workspace.store';
import { buildOption } from '../../metrics/chart-option';
import { ChartTokens } from '../../metrics/chart-theme';
import { activeLocale } from '../../i18n/locale';

/** `SourceContext` of every event the process monitor writes (debrief-monitor `SOURCE_CONTEXT`). */
export const MONITOR_SOURCE_CONTEXT = 'Debrief.Monitor';
/** Field that marks the monitor's per-process sample events. */
export const MONITOR_SAMPLE_FIELD = 'CpuPercent';
/** One series per tracked process instance: names repeat (globs), process ids do not. */
export const MONITOR_SPLIT = ['ProcessName', 'ProcessId'];
/** Buckets asked of the engine per chart: about one per pixel column of a small multiple. */
export const MONITOR_BUCKETS = 150;

/** The time windows the live panel offers, in minutes. */
export const MONITOR_WINDOWS = [5, 15, 60] as const;
export type MonitorWindow = (typeof MONITOR_WINDOWS)[number];

/**
 * Groups of small multiples: every process has the "process" fields; only .NET Framework processes sampled with the
 * opt-in runtime counters carry the "dotnet" ones, so that group is shown only when the engine finds them.
 */
export type MonitorGroup = 'process' | 'dotnet';

export const MONITOR_GROUPS: readonly { id: MonitorGroup; title: string }[] = [
  { id: 'process', title: $localize`:@@tools.monitor.group.process:Process` },
  { id: 'dotnet', title: $localize`:@@tools.monitor.group.dotnet:.NET runtime` },
];

/** A small multiple of the live panel: one sample field of the "Process stats" events. */
export interface MonitorMetric {
  id: string;
  group: MonitorGroup;
  title: string;
  /** The sample field charted (its per-bucket average). */
  field: string;
  unit: string;
}

export const MONITOR_METRICS: readonly MonitorMetric[] = [
  { id: 'cpu', group: 'process', title: $localize`:@@tools.monitor.metric.cpu:CPU`, field: 'CpuPercent', unit: '%' },
  { id: 'working-set', group: 'process', title: $localize`:@@tools.monitor.metric.workingSet:Working set`, field: 'WorkingSetMb', unit: 'MB' },
  { id: 'private', group: 'process', title: $localize`:@@tools.monitor.metric.privateBytes:Private bytes`, field: 'PrivateMb', unit: 'MB' },
  { id: 'handles', group: 'process', title: $localize`:@@tools.monitor.metric.handles:Handles`, field: 'Handles', unit: '' },
  { id: 'threads', group: 'process', title: $localize`:@@tools.monitor.metric.threads:Threads`, field: 'Threads', unit: '' },
  { id: 'io-read', group: 'process', title: $localize`:@@tools.monitor.metric.ioRead:IO read`, field: 'ReadBytesPerSec', unit: 'B/s' },
  { id: 'io-write', group: 'process', title: $localize`:@@tools.monitor.metric.ioWrite:IO write`, field: 'WriteBytesPerSec', unit: 'B/s' },
  { id: 'dotnet-heap', group: 'dotnet', title: $localize`:@@tools.monitor.metric.gcHeap:GC heap`, field: 'DotnetHeapBytes', unit: 'B' },
  { id: 'dotnet-gc-time', group: 'dotnet', title: $localize`:@@tools.monitor.metric.timeInGc:% time in GC`, field: 'DotnetTimeInGcPercent', unit: '%' },
  { id: 'dotnet-exceptions', group: 'dotnet', title: $localize`:@@tools.monitor.metric.exceptions:Exceptions`, field: 'DotnetExceptionsPerSec', unit: '/s' },
  { id: 'dotnet-contention', group: 'dotnet', title: $localize`:@@tools.monitor.metric.lockContention:Lock contention`, field: 'DotnetContentionPerSec', unit: '/s' },
];

/** The ad-hoc chart the engine computes for a metric. */
export function monitorChartDef(m: MonitorMetric): ChartDef {
  return { id: `monitor-${m.id}`, title: m.title, y: `avg(${m.field})`, bucket: 'auto', splitBy: MONITOR_SPLIT, type: 'line', unit: m.unit };
}

function normalise(path: string): string {
  return path.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
}

/**
 * Ids of the workspace sources inside the monitor's output folder (empty when unknown or not open). Ids, not names:
 * two runs write files of the same name into different folders, and only the id tells them apart.
 */
export function monitorSourceIds(summary: WorkspaceSummary | null, dir: string | null): number[] {
  if (!summary || !dir?.trim()) return [];
  const root = normalise(dir);
  return summary.sources.filter((s) => normalise(s.path) === root || normalise(s.path).startsWith(`${root}/`)).map((s) => s.id);
}

/** The workspace holds monitor samples (an opened monitor output, even without a run in this session). */
export function hasMonitorSamples(summary: WorkspaceSummary | null): boolean {
  return !!summary?.fields.some((f) => f.name === MONITOR_SAMPLE_FIELD && f.count > 0);
}

/**
 * The engine query for the live charts: the monitor's events, restricted to its output sources (by id) when they are
 * known, over a sliding window the engine resolves (the UI never computes "now").
 */
export function monitorQuery(sources: readonly number[], windowMin: MonitorWindow): Query {
  return {
    ...EMPTY_QUERY,
    filters: [{ field: 'SourceContext', op: 'eq', value: MONITOR_SOURCE_CONTEXT }],
    sources: [...sources],
    window: { kind: 'live', lastMs: windowMin * 60_000 },
  };
}

/** Slots (palette positions) that follow the entity: keys keep their slot once seen; new keys take the next free one. */
export function assignSlots(previous: Readonly<Record<string, number>>, charts: readonly (ChartData | null)[], slots: number): Record<string, number> {
  const next = { ...previous };
  let used = Object.keys(next).length;
  for (const c of charts) {
    for (const s of c?.series ?? []) {
      if (used >= slots) return next;
      if (s.key in next || s.key.startsWith('Other')) continue;
      next[s.key] = used++;
    }
  }
  return next;
}

/** Exact value with its unit, for the tooltip (axis labels stay compact). */
export function exactValue(v: unknown, unit: string): string {
  if (typeof v !== 'number' || !Number.isFinite(v)) return '—';
  const n = v.toLocaleString(activeLocale(), { maximumFractionDigits: 2 });
  return unit ? `${n} ${unit}` : n;
}

/** Units whose values the chart scales for people (KB, MB, GB, MB/s) instead of printing a fixed unit. */
export function isAutoUnit(unit: string): boolean {
  return unit === 'MB' || unit === 'B' || unit === 'B/s' || unit === '%';
}

/** A value humanized by its unit: bytes scale to KB/MB/GB, rates to MB/s, percentages follow the locale. */
export function humanValue(v: unknown, unit: string): string {
  if (typeof v !== 'number' || !Number.isFinite(v)) return '—';
  switch (unit) {
    case 'MB':
      return formatBytes(v * 1024 * 1024);
    case 'B':
      return formatBytes(v);
    case 'B/s':
      return v > 0 ? formatRate(v) : $localize`:@@tools.monitor.noRate:0 bytes/s`;
    case '%':
      return formatPercent(v, 1);
    case '/s':
      return `${formatNumber(v)}/s`;
    default:
      return formatNumber(v);
  }
}

/** Tooltip text: the humanized value, plus the exact one where scaling hides digits. */
export function tooltipValue(v: unknown, unit: string): string {
  const human = humanValue(v, unit);
  return unit === 'MB' || unit === 'B' || unit === 'B/s' ? `${human} (${exactValue(v, unit)})` : human;
}

/**
 * Direct labels for narrow small multiples: the process name alone (`OrderService.exe`) when it is unique in the chart,
 * the full `name · pid` key otherwise. Legend and tooltip always show the full key.
 */
export function endLabels(keys: readonly string[]): Map<string, string> {
  const short = (k: string) => k.split(' · ')[0] ?? k;
  const counts = new Map<string, number>();
  for (const k of keys) counts.set(short(k), (counts.get(short(k)) ?? 0) + 1);
  return new Map(keys.map((k) => [k, counts.get(short(k)) === 1 ? short(k) : k]));
}

export interface MonitorOptionInput {
  data: ChartData;
  tokens: ChartTokens;
  tz: 'utc' | 'local';
  slots: Readonly<Record<string, number>>;
  hidden: ReadonlySet<string>;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- ECharts series options are loose nested objects.
type Loose = Record<string, any>;

/**
 * A small multiple's ECharts option: the Metrics chart builder (end labels with moveOverlap, axis tooltip, lifecycle
 * lines) with colours that follow the process across charts (the series palette is CVD-safe for every pair, so lines
 * stay solid), the shared legend's toggles, and exact tooltip values. Only presentation: the values are the engine's.
 */
export function monitorOption({ data, tokens, tz, slots, hidden }: MonitorOptionInput): EChartsCoreOption {
  const slotOf = (key: string) => slots[key] ?? Number.MAX_SAFE_INTEGER;
  const series = [...data.series].sort((a, b) => slotOf(a.key) - slotOf(b.key));
  const base = buildOption({ ...data, series }, tokens, tz, new Map()) as Loose;
  const labels = endLabels(series.map((s) => s.key));
  const patched = (base['series'] as Loose[]).map((s): Loose => {
    const name = s['name'] as string;
    const endLabel = s['endLabel'] ? { ...s['endLabel'], formatter: labels.get(name) ?? name } : undefined;
    const slot = slots[name];
    if (slot === undefined) return { ...s, endLabel };
    return { ...s, endLabel, color: tokens.series[slot] ?? s['color'] };
  });
  // Every name explicitly, so a merged live update also turns a series back on.
  const selected = Object.fromEntries(patched.map((s) => [s['name'] as string, !hidden.has(s['name'] as string)]));
  return {
    ...base,
    grid: { ...base['grid'], top: 12 },
    // One shared legend (HTML, above the multiples) toggles a process in every chart.
    legend: { ...base['legend'], show: false, selected },
    tooltip: { ...base['tooltip'], valueFormatter: (v: unknown) => tooltipValue(v, data.def.unit) },
    yAxis: { ...base['yAxis'], axisLabel: { ...base['yAxis']?.axisLabel, formatter: (v: number) => humanValue(v, data.def.unit) } },
    series: patched,
  };
}
