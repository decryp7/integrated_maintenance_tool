import { ChangeDetectionStrategy, Component, computed, effect, inject, input, output, signal, untracked } from '@angular/core';
import type { EChartsCoreOption } from 'echarts/core';
import { ChartData, ChartDef, ChartSeries, LEVELS } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatDateTime, formatDuration, formatNumber } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { ChartTheme, ChartTokens } from './chart-theme';
import { EChart } from './echart';
import { TimelineZoom } from '../timeline/zoom';

const MAX_NAMED = 4;
const OTHER = 'Other';

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
  const named = series.slice(0, MAX_NAMED).map((s, i) => ({ name: s.key, color: t.series[i], points: s.points, other: false }));
  const rest = series.slice(MAX_NAMED);
  if (!rest.length) return named;
  if (def.type === 'line') {
    // Averages cannot be summed: draw the remainder as thin neutral lines sharing one legend entry.
    return [...named, ...rest.map((s) => ({ name: OTHER, color: t.other, points: s.points, other: true }))];
  }
  const sum = rest[0].points.map(([ts], i): [number, number | null] => {
    let total: number | null = null;
    for (const s of rest) {
      const v = s.points[i]?.[1];
      if (v != null) total = (total ?? 0) + v;
    }
    return [ts, total];
  });
  return [...named, { name: `${OTHER} (${rest.length})`, color: t.other, points: sum, other: true }];
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
    markLine: i === 0 ? markLine : undefined,
  }));

  return {
    animation: false,
    textStyle: { fontFamily: t.fontSans, color: t.muted },
    grid: { left: 56, right: directLabels ? 130 : 20, top: 30, bottom: 28 },
    legend: {
      type: 'scroll',
      top: 0,
      right: directLabels ? 130 : 20,
      textStyle: { color: t.text, fontFamily: t.fontMono, fontSize: 11 },
      icon: isLine ? 'roundRect' : 'rect',
      itemWidth: 14,
      itemHeight: isLine ? 3 : 8,
      pageTextStyle: { color: t.muted },
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
    series,
  };
}

@Component({
  selector: 'imt-chart-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [EChart, Icon],
  template: `
    <div class="head">
      <span class="title">{{ def().title }}</span>
      <span class="mono faint sub">{{ subtitle() }}</span>
      @if (loading()) { <span class="spinner" aria-label="Loading"></span> }
      <span class="spacer"></span>
      @if (lifecycleKinds().length) {
        <span class="lc faint">
          @for (k of lifecycleKinds(); track k.kind) {
            <span class="lc-item"><span class="dash" [style.border-color]="k.color"></span>{{ k.label }}</span>
          }
        </span>
      }
      @if (removable()) {
        <button class="icon-btn" [attr.aria-label]="'Remove chart ' + def().title" (click)="remove.emit()" [attr.title]="'Remove chart ' + def().title"><imt-icon name="close" [size]="14" /></button>
      }
    </div>
    @if (error(); as e) {
      <div class="msg err">{{ e }}</div>
    } @else if (empty()) {
      <div class="msg faint">No numeric values for {{ def().y }} in the current results.</div>
    } @else if (option(); as o) {
      <imt-echart class="chart" [options]="o" (brushed)="zoom.apply({ from: $event[0], to: $event[1] })" />
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; padding: 10px 14px 6px; border-bottom: 1px solid var(--border); min-height: 220px; }
    .head { display: flex; align-items: baseline; gap: 10px; }
    .title { font-weight: 600; }
    .sub { font-size: 11px; }
    .spacer { flex: 1; }
    .lc { display: flex; gap: 10px; font-size: 11px; }
    .lc-item { display: inline-flex; align-items: center; gap: 4px; }
    .dash { width: 0; height: 10px; border-left: 1px dashed; }
    .chart { flex: 1; min-height: 180px; }
    .msg { flex: 1; display: flex; align-items: center; justify-content: center; min-height: 160px; }
    .err { color: var(--danger-text); }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; align-self: center; }
    @keyframes spin { to { transform: rotate(360deg); } }
  `,
})
export class ChartCard {
  readonly def = input.required<ChartDef>();
  /** True for charts from the profile (queried by id); false for ad-hoc definitions. */
  readonly fromProfile = input(true);
  readonly removable = input(false);
  readonly remove = output<void>();

  protected readonly store = inject(WorkspaceStore);
  protected readonly zoom = inject(TimelineZoom);
  private readonly backend = inject(ImtBackend);
  private readonly theme = inject(ChartTheme);

  protected readonly data = signal<ChartData | null>(null);
  protected readonly loading = signal(false);
  protected readonly error = signal<string | null>(null);

  protected readonly empty = computed(() => {
    const d = this.data();
    return !!d && d.series.length === 0;
  });

  protected readonly subtitle = computed(() => {
    const d = this.def();
    const data = this.data();
    const bucket = data ? formatDuration(data.bucketMs) : d.bucket;
    const split = d.splitBy.length ? ` · split by ${d.splitBy.join(', ')}` : '';
    return `${d.y}${d.unit ? ' (' + d.unit + ')' : ''} · per ${bucket}${split}`;
  });

  /** Kinds present as dashed lines on this chart, for the header key. */
  protected readonly lifecycleKinds = computed(() => {
    const kinds = new Set((this.data()?.lifecycle ?? []).map((e) => e.kind));
    const tokens = this.theme.tokens();
    return [...kinds].map((kind) => ({
      kind,
      label: this.store.markerKindMap().get(kind)?.label ?? kind,
      color: tokens.markers[kind] ?? this.store.markerKindMap().get(kind)?.color ?? tokens.muted,
    }));
  });

  protected readonly option = computed(() => {
    const d = this.data();
    if (!d) return null;
    const labels = new Map([...this.store.markerKindMap().values()].map((k) => [k.kind, k.label]));
    return buildOption(d, this.theme.tokens(), this.store.timezone(), labels);
  });

  constructor() {
    let seq = 0;
    effect(() => {
      const def = this.def();
      const fromProfile = this.fromProfile();
      const query = this.store.effectiveQuery();
      const buckets = this.store.buckets();
      this.store.dataVersion();
      const mine = ++seq;
      untracked(() => {
        this.loading.set(true);
        this.backend
          .queryChart({ query, chartId: fromProfile ? def.id : null, def: fromProfile ? null : def, buckets })
          .then((d) => {
            if (mine !== seq) return;
            this.data.set(d);
            this.error.set(null);
          })
          .catch((e) => mine === seq && this.error.set(String(e)))
          .finally(() => mine === seq && this.loading.set(false));
      });
    });
  }
}
