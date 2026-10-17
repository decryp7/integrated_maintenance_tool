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
  const cols: HotspotColumn[] = [{ id: 'name', label: groupLabel, sort: { metric: null }, measurement: null, numeric: false, value: (r) => r.name, help: `Sort by ${groupLabel.toLowerCase()} name` }];
  const metricCol = (id: MetricId, of: 'self' | 'total', label: string, help: string): HotspotColumn | null =>
    has(m, id) ? { id: `${of}-${id}`, label, sort: { metric: id, of }, measurement: measured(m, id), numeric: true, value: (r) => formatMetric(r[of][id], id, m), help } : null;
  const pctMeasurement = measured(m, metric);
  const add = (c: HotspotColumn | null) => c && cols.push(c);
  add(metricCol('timeNs', 'self', 'Self time', 'Time spent in the function itself'));
  add(metricCol('timeNs', 'total', 'Total time', 'Time in the function and everything it called (each sample counted once)'));
  cols.push({ id: 'selfPct', label: 'Self %', sort: { metric, of: 'self' }, measurement: pctMeasurement, numeric: true, pct: 'self', value: (r) => formatPct(r.selfPct), help: 'Self share of the view total' });
  cols.push({ id: 'totalPct', label: 'Total %', sort: { metric, of: 'total' }, measurement: pctMeasurement, numeric: true, pct: 'total', value: (r) => formatPct(r.totalPct), help: 'Total (inclusive) share of the view total' });
  add(metricCol('samples', 'total', 'Samples', 'Stack samples that contain the function'));
  add(metricCol('calls', 'self', 'Calls', 'Number of calls'));
  if (has(m, 'calls') && has(m, 'timeNs')) {
    cols.push({ id: 'avg', label: 'Avg / call', sort: { metric: 'timeNs', of: 'avgPerCall' }, measurement: combineMeasurements(measured(m, 'calls'), measured(m, 'timeNs')), numeric: true, value: (r) => formatNs(r.avgPerCallNs), help: 'Total time divided by calls (rows without calls sort last)' });
  }
  const max = metricCol('maxCallNs', 'self', 'Max / call', 'Longest single call');
  if (max) add({ ...max, value: (r) => formatNs(r.maxPerCallNs ?? r.self.maxCallNs) });
  add(metricCol('allocCount', 'self', 'Allocs', 'Allocations made in the function itself'));
  add(metricCol('allocBytes', 'self', 'Alloc bytes', 'Bytes allocated in the function itself'));
  add(metricCol('exceptions', 'self', 'Exceptions', 'Exceptions thrown in the function itself'));
  add(metricCol('waitNs', 'self', 'Wait', 'Time blocked waiting (locks, I/O)'));
  return cols;
}

export const GROUP_LABELS = { function: 'Function', type: 'Type', namespace: 'Namespace', module: 'Module', thread: 'Thread' } as const;
