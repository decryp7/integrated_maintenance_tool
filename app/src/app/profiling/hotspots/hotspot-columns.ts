import { HotspotRow, Measurement, MetricDto, MetricId, ProfileSortSpec, ProfileSummary } from '../../core/api.types';
import { combineMeasurements, formatMetric, formatNs, formatPct } from '../profiling-format';

/** One column of the hotspots table. Pure description; the view renders it. */
export interface HotspotColumn {
  id: string;
  label: string;
  /** How the engine sorts by it; null = not sortable (the engine has no such sort key). */
  sort: ProfileSortSpec | null;
  /** Sampled (≈) or exact (=); null for the name. */
  measurement: Measurement | null;
  numeric: boolean;
  /** Share of the table's metric (rendered with a meter). */
  pct?: 'self' | 'total';
  value: (row: HotspotRow) => string;
  /** Plain-language description for the header tooltip. */
  help: string;
}

const has = (metrics: readonly MetricDto[], id: MetricId) => metrics.some((m) => m.id === id);
const measured = (metrics: readonly MetricDto[], id: MetricId): Measurement => metrics.find((m) => m.id === id)?.measurement ?? 'sampled';

/** Whether two sort specs mean the same engine sort. */
export function sameSort(a: ProfileSortSpec | null | undefined, b: ProfileSortSpec | null | undefined): boolean {
  if (!a || !b) return false;
  return (a.metric ?? null) === (b.metric ?? null) && (a.metric === null || (a.of ?? 'self') === (b.of ?? 'self'));
}

/**
 * The columns for a profile: only metrics the profile has, each tagged sampled or exact from the
 * engine's metric descriptions. `metric` is the table's metric (the one percentages refer to).
 */
export function hotspotColumns(profile: ProfileSummary, metric: MetricId, groupLabel: string): HotspotColumn[] {
  const m = profile.metrics;
  const cols: HotspotColumn[] = [{ id: 'name', label: groupLabel, sort: { metric: null }, measurement: null, numeric: false, value: (r) => r.name, help: $localize`:@@profiling.hotspots.help.name:Sort by ${groupLabel.toLowerCase()}:group: name` }];
  const metricCol = (id: MetricId, of: 'self' | 'total', label: string, help: string): HotspotColumn | null =>
    has(m, id) ? { id: `${of}-${id}`, label, sort: { metric: id, of }, measurement: measured(m, id), numeric: true, value: (r) => formatMetric(r[of][id], id, m), help } : null;
  const pctMeasurement = measured(m, metric);
  const add = (c: HotspotColumn | null) => c && cols.push(c);
  add(metricCol('timeNs', 'self', $localize`:@@profiling.hotspots.col.selfTime:Self time`, $localize`:@@profiling.hotspots.help.selfTime:Time spent in the function itself`));
  add(metricCol('timeNs', 'total', $localize`:@@profiling.hotspots.col.totalTime:Total time`, $localize`:@@profiling.hotspots.help.totalTime:Time in the function and everything it called (each sample counted once)`));
  cols.push({ id: 'selfPct', label: $localize`:@@profiling.hotspots.col.selfPct:Self %`, sort: { metric, of: 'self' }, measurement: pctMeasurement, numeric: true, pct: 'self', value: (r) => formatPct(r.selfPct), help: $localize`:@@profiling.hotspots.help.selfPct:Self share of the view total` });
  cols.push({ id: 'totalPct', label: $localize`:@@profiling.hotspots.col.totalPct:Total %`, sort: { metric, of: 'total' }, measurement: pctMeasurement, numeric: true, pct: 'total', value: (r) => formatPct(r.totalPct), help: $localize`:@@profiling.hotspots.help.totalPct:Total (inclusive) share of the view total` });
  add(metricCol('samples', 'total', $localize`:@@profiling.hotspots.col.samples:Samples`, $localize`:@@profiling.hotspots.help.samples:Stack samples that contain the function`));
  add(metricCol('calls', 'self', $localize`:@@profiling.hotspots.col.calls:Calls`, $localize`:@@profiling.hotspots.help.calls:Number of calls`));
  if (has(m, 'calls') && has(m, 'timeNs')) {
    cols.push({ id: 'avg', label: $localize`:@@profiling.hotspots.col.avg:Avg / call`, sort: { metric: 'timeNs', of: 'avgPerCall' }, measurement: combineMeasurements(measured(m, 'calls'), measured(m, 'timeNs')), numeric: true, value: (r) => formatNs(r.avgPerCallNs), help: $localize`:@@profiling.hotspots.help.avg:Total time divided by calls (rows without calls sort last)` });
  }
  const max = metricCol('maxCallNs', 'self', $localize`:@@profiling.hotspots.col.max:Max / call`, $localize`:@@profiling.hotspots.help.max:Longest single call`);
  if (max) add({ ...max, value: (r) => formatNs(r.maxPerCallNs ?? r.self.maxCallNs) });
  add(metricCol('allocCount', 'self', $localize`:@@profiling.hotspots.col.allocs:Allocs`, $localize`:@@profiling.hotspots.help.allocs:Allocations made in the function itself`));
  add(metricCol('allocBytes', 'self', $localize`:@@profiling.hotspots.col.allocBytes:Alloc bytes`, $localize`:@@profiling.hotspots.help.allocBytes:Bytes allocated in the function itself`));
  add(metricCol('exceptions', 'self', $localize`:@@profiling.hotspots.col.exceptions:Exceptions`, $localize`:@@profiling.hotspots.help.exceptions:Exceptions thrown in the function itself`));
  add(metricCol('waitNs', 'self', $localize`:@@profiling.hotspots.col.wait:Wait`, $localize`:@@profiling.hotspots.help.wait:Time blocked waiting (locks, I/O)`));
  return cols;
}

export const GROUP_LABELS = {
  function: $localize`:@@profiling.hotspots.group.function:Function`,
  type: $localize`:@@profiling.hotspots.group.type:Type`,
  namespace: $localize`:@@profiling.hotspots.group.namespace:Namespace`,
  module: $localize`:@@profiling.hotspots.group.module:Module`,
  thread: $localize`:@@profiling.hotspots.group.thread:Thread`,
} as const;

/** The lower-case noun of a grouping for "N functions" style counts: one and other forms. */
export const GROUP_NOUNS: Record<keyof typeof GROUP_LABELS, { one: string; other: string }> = {
  function: { one: $localize`:@@profiling.hotspots.noun.function.one:function`, other: $localize`:@@profiling.hotspots.noun.function.other:functions` },
  type: { one: $localize`:@@profiling.hotspots.noun.type.one:type`, other: $localize`:@@profiling.hotspots.noun.type.other:types` },
  namespace: { one: $localize`:@@profiling.hotspots.noun.namespace.one:namespace`, other: $localize`:@@profiling.hotspots.noun.namespace.other:namespaces` },
  module: { one: $localize`:@@profiling.hotspots.noun.module.one:module`, other: $localize`:@@profiling.hotspots.noun.module.other:modules` },
  thread: { one: $localize`:@@profiling.hotspots.noun.thread.one:thread`, other: $localize`:@@profiling.hotspots.noun.thread.other:threads` },
};
