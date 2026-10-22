import { Level, LEVELS, AnomalyKind } from '../core/api.types';
import { qlQuote } from './ql-text';

/** Display-only helpers. */

export const ANOMALY_LABELS: Record<AnomalyKind, string> = {
  errorSpike: $localize`:@@analysis.format.anomaly.errorSpike:Error spike`,
  silence: $localize`:@@analysis.format.anomaly.silence:Silence`,
  crash: $localize`:@@analysis.format.anomaly.crash:Crash`,
  slowOutlier: $localize`:@@analysis.format.anomaly.slowOutlier:Slow outliers`,
  rateChange: $localize`:@@analysis.format.anomaly.rateChange:Rate change`,
  memoryGrowth: $localize`:@@analysis.format.anomaly.memoryGrowth:Memory growth`,
};

/** Glyph per kind (colour is never the only cue). */
export const ANOMALY_ICONS: Record<AnomalyKind, string> = {
  errorSpike: '▲',
  silence: '◌',
  crash: '✖',
  slowOutlier: '◔',
  rateChange: '⇅',
  memoryGrowth: '↗',
};

export function anomalyLabel(kind: AnomalyKind): string {
  return ANOMALY_LABELS[kind] ?? kind;
}

export function anomalyIcon(kind: AnomalyKind): string {
  return ANOMALY_ICONS[kind] ?? '•';
}

/** "+12.5%" / "−3%" / "—". */
export function formatChange(pct: number | null | undefined): string {
  if (pct === null || pct === undefined || !Number.isFinite(pct)) return '—';
  const rounded = Math.round(pct * 10) / 10;
  return `${rounded > 0 ? '+' : rounded < 0 ? '−' : ''}${Math.abs(rounded)}%`;
}

export type Direction = 'up' | 'down' | 'flat';

export function direction(pct: number | null | undefined): Direction {
  if (pct === null || pct === undefined || !Number.isFinite(pct) || Math.abs(pct) < 0.05) return 'flat';
  return pct > 0 ? 'up' : 'down';
}

export const ARROWS: Record<Direction, string> = { up: '▲', down: '▼', flat: '–' };

/** Levels of a pattern as [level, count] pairs, empty levels omitted. */
export function levelCounts(counts: readonly number[]): { level: Level; count: number }[] {
  return LEVELS.map((level, i) => ({ level, count: counts[i] ?? 0 })).filter((l) => l.count > 0);
}

/** Width (0..100) of `value` against `max`, never invisible when non-zero. */
export function barWidth(value: number, max: number): number {
  if (value <= 0 || max <= 0) return 0;
  return Math.max(2, Math.min(100, (value / max) * 100));
}

export function shortId(id: string, max = 12): string {
  return id.length > max ? `${id.slice(0, max)}…` : id;
}

/** Query-language filter for a Serilog template or a mined `<*>` template. */
export function templateQl(template: string, mined: boolean): string {
  if (!mined) return `@template = ${qlQuote(template)}`;
  const regex = template
    .split('<*>')
    .map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('.+');
  return `@message ~ ${qlQuote(regex)}`;
}

/** "4.2×" (a lift: how many times more likely a value is in the subset). */
export function formatLift(lift: number): string {
  if (!Number.isFinite(lift)) return '—';
  return `${lift >= 100 ? Math.round(lift) : Math.round(lift * 10) / 10}×`;
}

/** "p < 0.001" or "p = 0.012". */
export function formatP(p: number): string {
  if (!Number.isFinite(p)) return '—';
  return p < 0.001 ? 'p < 0.001' : `p = ${p.toFixed(3)}`;
}

/** One bar of a sparkline: position and height in the unit square scaled by the caller. */
export interface SparkBar {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Bars for a trend in a `width` by `height` box: heights relative to the largest bucket, at least 1 for a non-empty
 * bucket (so a single event stays visible), none for an empty one.
 */
export function sparkBars(trend: readonly number[], width: number, height: number): SparkBar[] {
  const max = Math.max(0, ...trend);
  if (trend.length === 0 || max <= 0) return [];
  const step = width / trend.length;
  const bars: SparkBar[] = [];
  trend.forEach((n, i) => {
    if (n <= 0) return;
    const h = Math.max(1, Math.round((n / max) * height));
    bars.push({ x: Math.round(i * step * 100) / 100, y: height - h, width: Math.max(1, Math.round((step - 0.5) * 100) / 100), height: h });
  });
  return bars;
}

/** Index of the busiest bucket of a trend, or -1 for none. */
export function peakBucket(trend: readonly number[]): number {
  let best = -1;
  let max = 0;
  trend.forEach((n, i) => {
    if (n > max) {
      max = n;
      best = i;
    }
  });
  return best;
}

/** The query-language filter for events of a mined template whose message also contains `value`. */
export function templateValueQl(template: string, mined: boolean, value: string): string {
  return `(${templateQl(template, mined)}) and @message contains ${qlQuote(value)}`;
}
