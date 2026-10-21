import { CaptureSummary, HeapSummary, Measurement, MetricDto, MetricId, MetricUnit, ProfileSummary } from '../core/api.types';
import { formatCount } from '../core/format';
import { localizeDecimal } from '../i18n/locale';
import { formatBytes, formatExactBytes } from '../shared/format/units';

export { formatBytes };

/** Pure presentation helpers of the profiling views (units, labels, measurement tags). */

const UNIT_OF: Record<MetricId, MetricUnit> = {
  samples: 'count',
  timeNs: 'nanoseconds',
  calls: 'count',
  allocCount: 'count',
  allocBytes: 'bytes',
  exceptions: 'count',
  waitNs: 'nanoseconds',
  maxCallNs: 'nanoseconds',
};

export function unitOf(metric: MetricId, metrics: readonly MetricDto[] = []): MetricUnit {
  return metrics.find((m) => m.id === metric)?.unit ?? UNIT_OF[metric];
}

/** "850 ns", "12.5 µs", "3.20 ms", "1.25 s", "2 min 5 s". */
export function formatNs(ns: number | null | undefined): string {
  if (ns === null || ns === undefined || !Number.isFinite(ns)) return '—';
  const a = Math.abs(ns);
  if (a < 1_000) return `${Math.round(ns)} ns`;
  if (a < 1_000_000) return `${localizeDecimal((ns / 1_000).toFixed(1))} µs`;
  if (a < 1_000_000_000) return `${localizeDecimal((ns / 1_000_000).toFixed(a < 10_000_000 ? 2 : 1))} ms`;
  if (a < 60_000_000_000) return `${localizeDecimal((ns / 1_000_000_000).toFixed(2))} s`;
  const s = Math.round(ns / 1_000_000_000);
  return `${Math.floor(s / 60)} min ${s % 60} s`;
}

export function formatValue(value: number | null | undefined, unit: MetricUnit): string {
  if (value === null || value === undefined) return '—';
  if (unit === 'nanoseconds') return formatNs(value);
  if (unit === 'bytes') return formatBytes(value);
  return formatCount(Math.round(value));
}

export function formatMetric(value: number | null | undefined, metric: MetricId, metrics: readonly MetricDto[] = []): string {
  return formatValue(value, unitOf(metric, metrics));
}

export function formatPct(pct: number | null | undefined, digits = 1): string {
  if (pct === null || pct === undefined || !Number.isFinite(pct)) return '—';
  return `${localizeDecimal(pct.toFixed(digits))}%`;
}

/** Percentage points with a sign: "+12.3 pp", "−4.0 pp", "0.0 pp". */
export function formatPp(pp: number): string {
  const sign = pp > 0.005 ? '+' : pp < -0.005 ? '−' : '';
  return `${sign}${localizeDecimal(Math.abs(pp).toFixed(1))} pp`;
}

/** A signed byte or count delta: "+4.0 KB", "−3", "0". */
export function formatDelta(value: number, unit: 'bytes' | 'count'): string {
  const sign = value > 0 ? '+' : value < 0 ? '−' : '';
  return sign + (unit === 'bytes' ? formatBytes(Math.abs(value)) : formatCount(Math.abs(value)));
}

/** The exact signed byte delta for a tooltip: "+4,096 bytes". */
export function formatExactDelta(value: number): string {
  const sign = value > 0 ? '+' : value < 0 ? '−' : '';
  return sign + formatExactBytes(Math.abs(value));
}

/** The short glyph shown next to a value: ≈ for estimates, = for exact counts. */
export function measurementGlyph(m: Measurement): string {
  return m === 'sampled' ? '≈' : '=';
}

export function measurementLabel(m: Measurement): string {
  return m === 'sampled' ? $localize`:@@profiling.format.measurement.sampled:Sampled` : $localize`:@@profiling.format.measurement.exact:Exact`;
}

/** The tooltip explaining what the tag means. */
export function measurementHelp(m: Measurement): string {
  return m === 'sampled'
    ? $localize`:@@profiling.format.measurement.sampledHelp:Sampled (≈): a statistical estimate from periodic stack samples. Small values are noisy; compare shares rather than exact numbers.`
    : $localize`:@@profiling.format.measurement.exactHelp:Exact (=): every occurrence was recorded (instrumentation or events), so the value is a count, not an estimate.`;
}

/** Sampled when any input is sampled (a ratio of an estimate is an estimate). */
export function combineMeasurements(...ms: (Measurement | null | undefined)[]): Measurement {
  return ms.some((m) => m === 'sampled') ? 'sampled' : 'exact';
}

export function measurementOf(summary: ProfileSummary | null | undefined, metric: MetricId): Measurement | null {
  return summary?.metrics.find((m) => m.id === metric)?.measurement ?? null;
}

export function metricLabel(summary: ProfileSummary | null | undefined, metric: MetricId): string {
  return summary?.metrics.find((m) => m.id === metric)?.label ?? metric;
}

const KIND_LABEL: Record<ProfileSummary['kind'], string> = {
  cpu: $localize`:@@profiling.format.kind.cpu:CPU`,
  alloc: $localize`:@@profiling.format.kind.alloc:Allocations`,
  wall: $localize`:@@profiling.format.kind.wall:Wall clock`,
  mixed: $localize`:@@profiling.format.kind.mixed:Mixed`,
};

export function captureKindLabel(c: CaptureSummary): string {
  return c.capture === 'heap' ? $localize`:@@profiling.format.kind.heap:Heap snapshot` : KIND_LABEL[c.kind];
}

export function isProfile(c: CaptureSummary | null | undefined): c is ProfileSummary {
  return c?.capture === 'profile';
}

export function isHeap(c: CaptureSummary | null | undefined): c is HeapSummary {
  return c?.capture === 'heap';
}

/** "Shop.Orders.OrderService.Place" → "OrderService.Place" for narrow labels. */
export function shortName(name: string): string {
  const native = name.indexOf('!');
  if (native >= 0) return name.slice(native + 1);
  const parts = name.split('.');
  return parts.length > 2 ? parts.slice(-2).join('.') : name;
}
