import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, Injector, input, output, signal, untracked } from '@angular/core';
import type { EChartsCoreOption } from 'echarts/core';
import { ChartData, ChartDef, ChartSeries, LEVELS } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatDateTime, formatDuration, formatNumber } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { ChartTheme, ChartTokens } from './chart-theme';
import { EChart } from './echart';
import { TimelineZoom } from '../timeline/zoom';
import { TimeCursor, TimeCursorSource, TimeRange, timeToFrac } from '../analysis/time-cursor';
import { LiveTail } from '../workspace/live-tail';
import { VisualExportService, VisualFormat } from '../share/visuals/visual-export.service';
import { ReportActions } from '../share/report/report-actions';

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
    grid: { left: CHART_GRID.left, right: directLabels ? CHART_GRID.rightLabelled : CHART_GRID.right, top: CHART_GRID.top, bottom: CHART_GRID.bottom },
    legend: {
      type: 'scroll',
      top: 0,
      right: directLabels ? CHART_GRID.rightLabelled : CHART_GRID.right,
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
  imports: [EChart, Icon, TimeCursorSource],
  template: `
    <div class="head">
      <span class="title">{{ def().title }}</span>
      <span class="mono faint sub">{{ subtitle() }}</span>
      @if (loading()) { <span class="spinner" role="status" aria-label="Loading"></span> }
      <span class="spacer"></span>
      @if (lifecycleKinds().length) {
        <span class="lc faint">
          @for (k of lifecycleKinds(); track k.kind) {
            <span class="lc-item"><span class="dash" [style.border-color]="k.color"></span>{{ k.label }}</span>
          }
        </span>
      }
      <span class="actions">
        <button class="icon-btn small" type="button" [attr.aria-label]="'Add chart ' + def().title + ' to report'" title="Add to report" [disabled]="!option()" (click)="addToReport()"><imt-icon name="plus" [size]="13" /></button>
        <button class="icon-btn small" type="button" [attr.aria-label]="'Export chart ' + def().title + ' as PNG'" title="Export PNG" [disabled]="!option()" (click)="exportImage('png')">PNG</button>
        <button class="icon-btn small" type="button" [attr.aria-label]="'Export chart ' + def().title + ' as SVG'" title="Export SVG" [disabled]="!option()" (click)="exportImage('svg')">SVG</button>
      </span>
      @if (removable()) {
        <button class="icon-btn" [attr.aria-label]="'Remove chart ' + def().title" (click)="remove.emit()" [attr.title]="'Remove chart ' + def().title"><imt-icon name="close" [size]="14" /></button>
      }
    </div>
    @if (error(); as e) {
      <div class="msg err">{{ e }}</div>
    } @else if (empty()) {
      <div class="msg faint">No numeric values for {{ def().y }} in the current results.</div>
    } @else if (option(); as o) {
      <div class="plot" [imtTimeCursor]="range()" [imtTimeCursorLeft]="grid().left" [imtTimeCursorRight]="grid().right">
        <imt-echart class="chart" [options]="o" [merge]="live.enabled()" (brushed)="brushed($event)" />
        @if (hoverLeft(); as left) {
          <div class="hover-line" [style.left]="left" [style.top.px]="grid().top" [style.bottom.px]="grid().bottom" aria-hidden="true"></div>
        }
      </div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; padding: 10px 14px 6px; border-bottom: 1px solid var(--border); min-height: 220px; }
    .head { display: flex; align-items: baseline; gap: 10px; }
    .title { font-weight: 600; }
    .sub { font-size: 11px; }
    .spacer { flex: 1; }
    .actions { display: inline-flex; gap: 2px; align-self: center; }
    .small { font-size: 10px; font-family: var(--font-mono); min-width: 22px; }
    .lc { display: flex; gap: 10px; font-size: 11px; }
    .lc-item { display: inline-flex; align-items: center; gap: 4px; }
    .dash { width: 0; height: 10px; border-left: 1px dashed; }
    .plot { flex: 1; min-height: 180px; position: relative; display: flex; flex-direction: column; }
    .chart { flex: 1; min-height: 180px; }
    .hover-line { position: absolute; width: 0; border-left: 1px solid var(--border-3); pointer-events: none; }
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
  private readonly cursor = inject(TimeCursor);
  private readonly backend = inject(ImtBackend);
  private readonly theme = inject(ChartTheme);
  protected readonly live = inject(LiveTail);
  /** The card is on screen; live refreshes of cards scrolled out of view wait until they are visible again. */
  private readonly onScreen = signal(true);

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

  /** The time range the x axis spans. */
  protected readonly range = computed<TimeRange | null>(() => {
    const d = this.data();
    return d ? { from: d.from, to: d.to } : null;
  });

  /** The plot box insets, as `buildOption` laid them out. */
  protected readonly grid = computed(() => {
    const g = (this.option() as { grid?: { left: number; right: number; top: number; bottom: number } } | null)?.grid;
    return g ?? { left: CHART_GRID.left, right: CHART_GRID.right, top: CHART_GRID.top, bottom: CHART_GRID.bottom };
  });

  /** CSS `left` of the shared time cursor's line (hovered in any time-based view), or null. */
  protected readonly hoverLeft = computed(() => {
    const frac = timeToFrac(this.cursor.hovered(), this.range());
    if (frac === null) return null;
    const { left, right } = this.grid();
    return `calc(${left}px + (100% - ${left + right}px) * ${frac.toFixed(5)})`;
  });

  /** Resolves the export and report services on first use (they need the share and analysis backends). */
  private readonly injector = inject(Injector);

  /** Saves what the chart shows as an image (theme-aware; re-rendered off screen). */
  protected exportImage(kind: VisualFormat): void {
    const o = this.option();
    if (o) void this.injector.get(VisualExportService).exportChart(o, this.def().title, kind);
  }

  /** Adds the chart (its definition and an SVG snapshot) to the active report. */
  protected addToReport(): void {
    void this.injector.get(ReportActions).addChart(this.def(), this.option());
  }

  /** A brush selects the range (for "Analyse this moment") and zooms to it. */
  protected brushed([from, to]: [number, number]): void {
    this.cursor.select({ from, to });
    this.zoom.apply({ from, to });
  }

  protected readonly option = computed(() => {
    const d = this.data();
    if (!d) return null;
    const labels = new Map([...this.store.markerKindMap().values()].map((k) => [k.kind, k.label]));
    const option = buildOption(d, this.theme.tokens(), this.store.timezone(), labels);
    return this.live.enabled() ? liveOption(option) : option;
  });

  constructor() {
    let seq = 0;
    let fetched = '';
    let fetchedKey = '';
    effect(() => {
      const def = this.def();
      const fromProfile = this.fromProfile();
      const query = this.store.effectiveQuery();
      const buckets = this.store.buckets();
      const version = this.store.dataVersion();
      const onScreen = this.onScreen();
      untracked(() => {
        const key = JSON.stringify([def, fromProfile, query, buckets]);
        const fingerprint = `${key}|${version}`;
        if (fingerprint === fetched) return;
        // A live refresh of a chart scrolled out of view waits until it is visible (the engine buckets anyway).
        if (!onScreen && key === fetchedKey && this.live.isLiveRefresh()) return;
        fetched = fingerprint;
        fetchedKey = key;
        const mine = ++seq;
        // Live updates replace the data in place; the spinner would only flicker.
        if (!this.data() || !this.live.isLiveRefresh()) this.loading.set(true);
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

    const host = inject<ElementRef<HTMLElement>>(ElementRef);
    const destroyRef = inject(DestroyRef);
    afterNextRender(() => {
      if (typeof IntersectionObserver === 'undefined') return;
      const io = new IntersectionObserver((entries) => this.onScreen.set(entries.some((e) => e.isIntersecting)));
      io.observe(host.nativeElement);
      destroyRef.onDestroy(() => io.disconnect());
    });
  }
}
