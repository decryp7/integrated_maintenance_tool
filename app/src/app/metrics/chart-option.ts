import type { EChartsCoreOption } from 'echarts/core';
import { ChartData, ChartDef, ChartSeries, LEVELS } from '../core/api.types';
import { formatDateTime, formatNumber } from '../core/format';
import { ChartTokens } from './chart-theme';

// The pure part of a chart: its ECharts option from the engine's data. Kept apart from the `ChartCard` component so that
// code which only builds options (dashboards, the monitor, the browser mocks) does not load the component or ECharts.

/** Live charts animate their updates briefly (none with reduced motion). */
export const LIVE_UPDATE_MS = 200;

function reducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** Option for a live chart: the same chart, with short update transitions so merged updates glide. */
export function liveOption(option: EChartsCoreOption, reduced = reducedMotion()): EChartsCoreOption {
  return reduced ? option : { ...option, animation: true, animationDuration: 0, animationDurationUpdate: LIVE_UPDATE_MS, animationEasingUpdate: 'linear' };
}

const MAX_NAMED = 4;
/** Plot box of every chart (ECharts `grid`), px; the right inset grows when series are labelled directly. */
export const CHART_GRID = { left: 56, top: 30, bottom: 28, right: 20, rightLabelled: 130 } as const;
/** Buckets of a chart whose width is not known (not laid out). */
export const CHART_DEFAULT_BUCKETS = 400;
const OTHER = $localize`:@@metrics.chart.other:Other`;
const LEGEND_PER_ROW = 4;
const LEGEND_ROW_PX = 20;

interface Prepared {
  name: string;
  color: string;
  points: [number, number | null][];
  other: boolean;
}

/** Keeps the first four series in the validated palette and folds the rest into a neutral "Other". */
function prepare(def: ChartDef, series: ChartSeries[], t: ChartTokens): Prepared[] {
  const byLevel = def.splitBy.length === 1 && def.splitBy[0] === '@level';
  if (byLevel) {
    return LEVELS.map((l) => series.find((s) => s.key === l))
      .filter((s): s is ChartSeries => !!s)
      .map((s) => ({ name: s.key, color: t.levels[s.key] ?? t.other, points: s.points, other: false }));
  }
  const named = series.slice(0, MAX_NAMED).map((s, i) => ({ name: s.key, color: t.series[i] ?? t.other, points: s.points, other: false }));
  const rest = series.slice(MAX_NAMED);
  const firstRest = rest[0];
  if (!firstRest) return named;
  if (def.type === 'line') {
    // Averages cannot be summed: draw the remainder as thin neutral lines sharing one legend entry.
    return [...named, ...rest.map((s) => ({ name: OTHER, color: t.other, points: s.points, other: true }))];
  }
  const sum = firstRest.points.map(([ts], i): [number, number | null] => {
    let total: number | null = null;
    for (const s of rest) {
      const v = s.points[i]?.[1];
      if (v != null) total = (total ?? 0) + v;
    }
    return [ts, total];
  });
  return [...named, { name: $localize`:@@metrics.chart.otherCount:Other (${rest.length}:count:)`, color: t.other, points: sum, other: true }];
}

export function buildOption(data: ChartData, t: ChartTokens, tz: 'utc' | 'local', kindLabels: Map<string, string>): EChartsCoreOption {
  const def = data.def;
  const prepared = prepare(def, data.series, t);
  const stacked = def.type === 'stacked';
  const isLine = def.type === 'line';
  const directLabels = isLine && prepared.filter((p) => !p.other).length <= MAX_NAMED;
  const unit = def.unit ? ` ${def.unit}` : '';

  const markLine = {
    silent: true,
    symbol: ['none', 'none'],
    animation: false,
    data: data.lifecycle.slice(0, 200).map((e) => ({
      xAxis: e.ts,
      lineStyle: { color: t.markers[e.kind] ?? t.muted, type: 'dashed', width: 1, opacity: 0.8 },
      label: { show: false },
      name: `${kindLabels.get(e.kind) ?? e.kind} · ${e.group}`,
    })),
  };

  const series = prepared.map((p, i) => ({
    name: p.name,
    type: isLine ? 'line' : 'bar',
    stack: stacked ? 'total' : undefined,
    data: p.points,
    showSymbol: false,
    connectNulls: false,
    sampling: isLine ? 'lttb' : undefined,
    barCategoryGap: '20%',
    color: p.color,
    lineStyle: isLine ? { width: p.other ? 1 : 2, opacity: p.other ? 0.6 : 1 } : undefined,
    itemStyle: !isLine ? { borderRadius: stacked ? 0 : [2, 2, 0, 0] } : undefined,
    emphasis: { focus: 'series' },
    endLabel: directLabels && !p.other ? { show: true, formatter: '{a}', color: t.text, fontSize: 10.5, fontFamily: t.fontMono } : undefined,
    labelLayout: directLabels && !p.other ? { moveOverlap: 'shiftY', hideOverlap: true } : undefined,
    markLine: i === 0 ? markLine : undefined,
  }));
  // The legend wraps (never pages); reserve a row of headroom per ~LEGEND_PER_ROW entries so it cannot overlap the plot.
  const legendRows = Math.max(1, Math.ceil(new Set(prepared.map((p) => p.name)).size / LEGEND_PER_ROW));
  const gridTop = CHART_GRID.top + (legendRows - 1) * LEGEND_ROW_PX;

  return {
    animation: false,
    textStyle: { fontFamily: t.fontSans, color: t.muted },
    grid: { left: CHART_GRID.left, right: directLabels ? CHART_GRID.rightLabelled : CHART_GRID.right, top: gridTop, bottom: CHART_GRID.bottom },
    legend: {
      type: 'plain',
      left: CHART_GRID.left,
      top: 0,
      right: directLabels ? CHART_GRID.rightLabelled : CHART_GRID.right,
      textStyle: { color: t.text, fontFamily: t.fontMono, fontSize: 11 },
      icon: isLine ? 'roundRect' : 'rect',
      itemWidth: 14,
      itemHeight: isLine ? 3 : 8,
    },
    tooltip: {
      trigger: 'axis',
      backgroundColor: t.tooltipBg,
      borderColor: t.border,
      textStyle: { color: t.text, fontSize: 12 },
      axisPointer: { type: 'line', lineStyle: { color: t.border } },
      valueFormatter: (v: unknown) => (typeof v === 'number' ? `${formatNumber(v)}${unit}` : '—'),
    },
    xAxis: {
      type: 'time',
      min: data.from,
      max: data.to,
      axisLine: { lineStyle: { color: t.grid } },
      axisTick: { show: false },
      splitLine: { show: false },
      axisLabel: {
        color: t.faint,
        fontFamily: t.fontMono,
        fontSize: 10,
        hideOverlap: true,
        formatter: (v: number) => formatDateTime(v, tz).split(' ').slice(2).join(' ').slice(0, 5),
      },
      axisPointer: { label: { formatter: (p: { value: number }) => formatDateTime(p.value, tz, true) } },
    },
    yAxis: {
      type: 'value',
      splitNumber: 3,
      axisLabel: { color: t.faint, fontFamily: t.fontMono, fontSize: 10, formatter: (v: number) => formatNumber(v) },
      splitLine: { lineStyle: { color: t.grid } },
    },
    brush: { xAxisIndex: 0, brushType: 'lineX', brushMode: 'single', throttleType: 'debounce', brushStyle: { color: t.accentSoft, borderColor: t.accent, borderWidth: 1 }, outOfBrush: { colorAlpha: 1 } },
    // Brushing is armed programmatically (see EChart), not from toolbox buttons.
    toolbox: { show: false },
    series,
  };
}
