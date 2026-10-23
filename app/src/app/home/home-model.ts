/**
 * The Home dashboard's pure view helpers: sparkline geometry, the history chart's ECharts option, the time range of a
 * history bucket and the recent-activity feed. Every number comes from the engine (`HomeSummary`); these only lay it
 * out.
 */
import type { EChartsCoreOption } from 'echarts/core';
import { HomeSummary, Level, LEVELS } from '../core/api.types';
import { formatDateTime, formatNumber, TimeZoneMode } from '../core/format';
import type { Breadcrumb } from '../core/activity/types';
import type { ChartTokens } from '../metrics/chart-theme';

/** The history buckets Home asks for. */
export const HOME_BUCKETS = 60;
/** Least time between two refreshes pushed by imports, live tail or profile changes. */
export const HOME_REFRESH_MS = 2000;
/** Entries the recent activity feed shows. */
export const FEED_SIZE = 8;

/** The sparkline's drawing space (viewBox units; CSS scales it). */
export const SPARK_W = 100;
export const SPARK_H = 24;

/** An SVG path (line) through `values`, scaled to the largest; '' with fewer than two points. */
export function sparklinePath(values: readonly number[], w = SPARK_W, h = SPARK_H): string {
  if (values.length < 2) return '';
  const max = Math.max(1, ...values);
  const step = w / (values.length - 1);
  // 1 unit of headroom at both edges so the 2px line is not clipped.
  const y = (v: number) => (h - 1 - (Math.max(0, v) / max) * (h - 2)).toFixed(2);
  return values.map((v, i) => `${i ? 'L' : 'M'}${(i * step).toFixed(2)} ${y(v)}`).join('');
}

/** The time range of history bucket `index` (the click target that sets the time range); null outside the history. */
export function bucketRange(summary: Pick<HomeSummary, 'from' | 'bucketMs' | 'histogram'>, index: number): { from: number; to: number } | null {
  if (summary.from === null || summary.bucketMs <= 0 || !Number.isInteger(index) || index < 0 || index >= summary.histogram.length) return null;
  const from = Math.floor(summary.from + index * summary.bucketMs);
  return { from, to: Math.max(from + 1, Math.ceil(summary.from + (index + 1) * summary.bucketMs)) };
}

/** The bucket a time falls in (the chart reports clicks as times). */
export function bucketAt(summary: Pick<HomeSummary, 'from' | 'bucketMs' | 'histogram'>, ts: number): number {
  if (summary.from === null || summary.bucketMs <= 0) return -1;
  return Math.min(summary.histogram.length - 1, Math.floor((ts - summary.from) / summary.bucketMs));
}

/** Height of the marker lane under the history plot, in px. */
const MARKER_LANE = 14;

/** Shape of a marker on the lane: shape tells the kind apart besides colour. */
function markerSymbol(kind: string): string {
  if (/crash|fatal|exception/i.test(kind)) return 'diamond';
  if (/warn/i.test(kind)) return 'triangle';
  if (/start|stop/i.test(kind)) return 'rect';
  return 'circle';
}

/** Levels in the history chart's stack, from the baseline: the most severe at the bottom, never hidden by info. */
const STACK: readonly Level[] = ['Fatal', 'Error', 'Warning', 'Information', 'Debug', 'Verbose'];

export interface HistoryLabels {
  level: (level: Level) => string;
  marker: (kind: string) => string;
  /** Name of the marker lane under the plot (tooltip and legend-free). */
  markerLane: string;
  anomaly: string;
}

/**
 * The history chart: events per bucket stacked by level (level colours, the timeline's), markers on a lane under the plot
 * and anomalies as shaded ranges. One axis, a legend (≥ 2 series), an axis tooltip; drag across it to select a
 * range (`EChart` arms brushing).
 */
export function historyOption(summary: HomeSummary, t: ChartTokens, tz: TimeZoneMode, labels: HistoryLabels): EChartsCoreOption | null {
  if (summary.from === null || summary.to === null || !summary.histogram.length) return null;
  const from = summary.from;
  const mid = (i: number) => Math.round(from + (i + 0.5) * summary.bucketMs);
  const present = STACK.filter((level) => summary.histogram.some((b) => (b[LEVELS.indexOf(level)] ?? 0) > 0));
  const markArea = {
    silent: false,
    itemStyle: { color: t.accentSoft, opacity: 0.3 },
    label: { show: false },
    data: summary.anomalies.map((a) => [{ xAxis: a.from, name: `${labels.anomaly}: ${a.message}` }, { xAxis: Math.max(a.to, a.from + 1) }]),
  };
  const series = present.map((level, i) => ({
    name: labels.level(level),
    type: 'bar',
    stack: 'events',
    barCategoryGap: '20%',
    color: t.levels[level],
    // Rounded data end on the top segment only.
    itemStyle: { borderRadius: i === present.length - 1 ? [2, 2, 0, 0] : 0 },
    emphasis: { focus: 'series' },
    data: summary.histogram.map((b, x) => [mid(x), b[LEVELS.indexOf(level)] ?? 0]),
    markArea: i === 0 ? markArea : undefined,
  }));
  // Markers sit on their own lane under the plot, shape-coded, never as rules through the bars.
  const markers = {
    name: labels.markerLane,
    type: 'scatter',
    xAxisIndex: 1,
    yAxisIndex: 1,
    symbolSize: 9,
    z: 4,
    data: summary.markers.map((m) => ({
      value: [m.ts, 0],
      symbol: markerSymbol(m.kind),
      itemStyle: { color: t.markers[m.kind] ?? t.muted },
      tooltip: { formatter: `${labels.marker(m.kind)} · ${formatDateTime(m.ts, tz)}${m.count > 1 ? ` ×${m.count}` : ''}` },
    })),
  };
  const laneH = summary.markers.length ? MARKER_LANE : 0;
  return {
    animation: false,
    textStyle: { fontFamily: t.fontSans, color: t.muted },
    // The legend wraps (never pages): headroom for a row per four entries, so it cannot overlap the plot.
    grid: [
      { left: 48, right: 16, top: 12 + Math.ceil(present.length / 4) * 20, bottom: 28 + laneH },
      { left: 48, right: 16, height: laneH, bottom: 4 },
    ],
    legend: { type: 'plain', data: present.map((l) => labels.level(l)), left: 48, top: 0, right: 16, textStyle: { color: t.text, fontSize: 11 }, icon: 'rect', itemWidth: 12, itemHeight: 8 },
    tooltip: {
      trigger: 'axis',
      backgroundColor: t.tooltipBg,
      borderColor: t.border,
      textStyle: { color: t.text, fontSize: 12 },
      axisPointer: { type: 'shadow', shadowStyle: { color: t.accentSoft, opacity: 0.4 } },
      valueFormatter: (v: unknown) => (typeof v === 'number' ? formatNumber(v) : '—'),
    },
    xAxis: [
      {
      type: 'time',
      min: from,
      max: summary.to,
      axisLine: { lineStyle: { color: t.grid } },
      axisTick: { show: false },
      splitLine: { show: false },
      axisLabel: { color: t.faint, fontSize: 11, hideOverlap: true, formatter: (v: number) => formatDateTime(v, tz).split(' ').slice(2).join(' ').slice(0, 5) },
      axisPointer: { label: { formatter: (p: { value: number }) => formatDateTime(p.value, tz) } },
      },
      { type: 'time', gridIndex: 1, min: from, max: summary.to, show: false },
    ],
    yAxis: [
      { type: 'value', splitNumber: 3, axisLabel: { color: t.faint, fontSize: 11, formatter: (v: number) => formatNumber(v) }, splitLine: { lineStyle: { color: t.grid, type: 'dashed' } } },
      { type: 'value', gridIndex: 1, min: -1, max: 1, show: false },
    ],
    brush: { xAxisIndex: [0, 1], brushType: 'lineX', brushMode: 'single', throttleType: 'debounce', brushStyle: { color: t.accentSoft, borderColor: t.accent, borderWidth: 1 }, outOfBrush: { colorAlpha: 1 } },
    toolbox: { show: false },
    series: summary.markers.length ? [...series, markers] : series,
  };
}

/** One line of the recent activity feed. */
export interface FeedItem {
  key: string;
  time: string;
  label: string;
  failed: boolean;
}

/** Bookkeeping entries the feed leaves out: they would drown the user's actions. */
const NOISE = /^(selection\.|search\.change$|query\.change$|filter\.change$|dock\.(autoHide|pin)$|timeRange\.)/;

/** The latest activity, newest first: the user's and the engine's actions, labelled by `label` (e.g. command titles). */
export function feedItems(crumbs: readonly Breadcrumb[], label: (c: Breadcrumb) => string, n = FEED_SIZE): FeedItem[] {
  return crumbs
    .filter((c) => !NOISE.test(c.operation))
    .slice(-n)
    .reverse()
    .map((c, i) => ({ key: `${c.operationId ?? c.time}:${i}`, time: c.time, label: label(c), failed: c.outcome === 'error' }));
}
