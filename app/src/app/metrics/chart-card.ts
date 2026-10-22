import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, Injector, input, output, signal, untracked } from '@angular/core';
import type { EChartsCoreOption } from 'echarts/core';
import { ChartData, ChartDef, ChartSeries, LEVELS } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { formatDateTime, formatDuration, formatNumber } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { ChartTheme, ChartTokens } from './chart-theme';
import { ChartPointMenu, EChart } from './echart';
import { ContextMenuService } from '../ui/context-menu.service';
import { anchors, NoteActions } from '../notes/note-actions';
import { TimelineZoom } from '../timeline/zoom';
import { TimeCursor, TimeCursorLine, TimeCursorSource, TimeRange } from '../analysis/time-cursor';
import { LiveTail } from '../workspace/live-tail';
import { VisualExportService, VisualFormat } from '../share/visuals/visual-export.service';
import { ReportActions } from '../share/report/report-actions';
import { bucketsForWidth, RESIZE_SETTLE_MS } from '../shared/bucket-count';
import { Debouncer } from '../shared/throttle';

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
    labelLayout: directLabels && !p.other ? { moveOverlap: 'shiftY' } : undefined,
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

@Component({
  selector: 'debrief-chart-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [EChart, Icon, TimeCursorSource, TimeCursorLine],
  template: `
    <div class="head">
      <span class="title">{{ def().title }}</span>
      <span class="mono faint sub">{{ subtitle() }}</span>
      @if (loading()) { <span class="spinner" role="status" aria-label="Loading" i18n-aria-label="@@metrics.chart.loading"></span> }
      <span class="spacer"></span>
      @if (lifecycleKinds().length) {
        <span class="lc faint">
          @for (k of lifecycleKinds(); track k.kind) {
            <span class="lc-item"><span class="dash" [style.border-color]="k.color"></span>{{ k.label }}</span>
          }
        </span>
      }
      <span class="actions">
        <button class="icon-btn small" type="button" [attr.aria-label]="addLabel()" title="Add to report" i18n-title="@@metrics.chart.addToReport.title" [disabled]="!option()" (click)="addToReport()"><debrief-icon name="plus" [size]="13" /></button>
        <button class="icon-btn small" type="button" [attr.aria-label]="exportLabelPng()" title="Export PNG" i18n-title="@@metrics.chart.exportPng.title" [disabled]="!option()" (click)="exportImage('png')">PNG</button>
        <button class="icon-btn small" type="button" [attr.aria-label]="exportLabelSvg()" title="Export SVG" i18n-title="@@metrics.chart.exportSvg.title" [disabled]="!option()" (click)="exportImage('svg')">SVG</button>
      </span>
      @if (removable()) {
        <button class="icon-btn" [attr.aria-label]="removeLabel()" (click)="remove.emit()" [attr.title]="removeLabel()"><debrief-icon name="close" [size]="14" /></button>
      }
    </div>
    @if (error(); as e) {
      <div class="msg err">{{ e }}</div>
    } @else if (empty()) {
      <div class="msg faint" i18n="@@metrics.chart.empty">No numeric values for {{ def().y }} in the current results.</div>
    } @else if (option(); as o) {
      <div class="plot" [debriefTimeCursor]="range()" [debriefTimeCursorLeft]="grid().left" [debriefTimeCursorRight]="grid().right">
        <debrief-echart class="chart" [options]="o" [merge]="live.enabled()" (brushed)="brushed($event)" (pointMenu)="pointMenu($event)" />
        <div
          class="hover-line"
          [debriefTimeCursorLine]="range()"
          [debriefTimeCursorLineLeft]="grid().left"
          [debriefTimeCursorLineRight]="grid().right"
          [style.top.px]="grid().top"
          [style.bottom.px]="grid().bottom"
          aria-hidden="true"
        ></div>
      </div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; padding: 10px 14px 6px; border-bottom: 1px solid var(--border); min-height: 220px; }
    .head { display: flex; align-items: baseline; gap: 10px; }
    .title { font-weight: 600; }
    .sub { font-size: var(--fs-xs); }
    .spacer { flex: 1; }
    .actions { display: inline-flex; gap: 2px; align-self: center; }
    .small { font-size: var(--fs-2xs); font-family: var(--font-mono); min-width: 22px; }
    .lc { display: flex; gap: 10px; font-size: var(--fs-xs); }
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
  private readonly backend = inject(DebriefBackend);
  private readonly theme = inject(ChartTheme);
  protected readonly live = inject(LiveTail);
  /** The card is on screen; live refreshes of cards scrolled out of view wait until they are visible again. */
  private readonly onScreen = signal(true);
  /** Buckets this chart asks for (from its own width; null until it is laid out). */
  private readonly buckets = signal<number | null>(null);

  protected readonly data = signal<ChartData | null>(null);
  protected readonly loading = signal(false);
  protected readonly error = signal<string | null>(null);

  protected readonly addLabel = computed(() => $localize`:@@metrics.chart.addToReport.aria:Add chart ${this.def().title}:title: to report`);
  protected readonly exportLabelPng = computed(() => $localize`:@@metrics.chart.exportPng.aria:Export chart ${this.def().title}:title: as PNG`);
  protected readonly exportLabelSvg = computed(() => $localize`:@@metrics.chart.exportSvg.aria:Export chart ${this.def().title}:title: as SVG`);
  protected readonly removeLabel = computed(() => $localize`:@@metrics.chart.remove:Remove chart ${this.def().title}:title:`);

  protected readonly empty = computed(() => {
    const d = this.data();
    return !!d && d.series.length === 0;
  });

  protected readonly subtitle = computed(() => {
    const d = this.def();
    const data = this.data();
    const bucket = data ? formatDuration(data.bucketMs) : d.bucket;
    const y = `${d.y}${d.unit ? ' (' + d.unit + ')' : ''}`;
    if (!d.splitBy.length) return $localize`:@@metrics.chart.subtitle:${y}:y: · per ${bucket}:bucket:`;
    const fields = d.splitBy.join(', ');
    return $localize`:@@metrics.chart.subtitleSplit:${y}:y: · per ${bucket}:bucket: · split by ${fields}:fields:`;
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

  /** Right-click on the chart: a note on the point (or on the moment of the plot under the pointer). */
  protected pointMenu(p: ChartPointMenu): void {
    const title = this.def().title;
    const notes = this.injector.get(NoteActions);
    const label =
      p.series === null
        ? $localize`:@@metrics.chart.noteMoment:Add note at this moment…`
        : $localize`:@@metrics.chart.notePoint:Add note to this point (${p.series}:series:)…`;
    this.injector.get(ContextMenuService).open(p.x, p.y, [{ label, run: () => void notes.write(anchors.chartPoint(title, p.series, p.at, p.value)) }]);
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
      const buckets = this.buckets();
      const version = this.store.dataVersion();
      const onScreen = this.onScreen();
      if (buckets === null) return;
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
    // The chart's own resolution, from its own width: resizing the timeline (or this card) does not re-query every chart,
    // and a resize only re-queries once it settles, in coarse steps.
    const resized = new Debouncer(RESIZE_SETTLE_MS);
    destroyRef.onDestroy(() => resized.cancel());
    afterNextRender(() => {
      const plotWidth = () => host.nativeElement.getBoundingClientRect().width - CHART_GRID.left - CHART_GRID.right;
      const measure = () => {
        const width = plotWidth();
        return width > 0 ? bucketsForWidth(width) : CHART_DEFAULT_BUCKETS;
      };
      this.buckets.set(measure());
      if (typeof ResizeObserver === 'undefined') return;
      const ro = new ResizeObserver(() => resized.run(() => this.buckets.set(measure())));
      ro.observe(host.nativeElement);
      destroyRef.onDestroy(() => ro.disconnect());
    });
    afterNextRender(() => {
      if (typeof IntersectionObserver === 'undefined') return;
      const io = new IntersectionObserver((entries) => this.onScreen.set(entries.some((e) => e.isIntersecting)));
      io.observe(host.nativeElement);
      destroyRef.onDestroy(() => io.disconnect());
    });
  }
}
