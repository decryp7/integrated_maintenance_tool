import { afterNextRender, ChangeDetectionStrategy, Component, DestroyRef, effect, ElementRef, inject, input, output } from '@angular/core';
import type { ECharts, EChartsCoreOption } from 'echarts/core';
import { ECHARTS_INIT } from './echarts-init';
import { spreadLabels } from './label-spread';

interface BrushEndEvent {
  areas?: { coordRange?: [number, number] }[];
}

/** What ECharts passes to a series item's `contextmenu` handler (the parts Debrief reads). */
interface ItemMenuEvent {
  componentType?: string;
  seriesName?: string;
  value?: unknown;
  event?: { event?: MouseEvent };
}

/** What zrender passes for a right-click anywhere on the canvas. */
interface ZrMenuEvent {
  offsetX: number;
  offsetY: number;
  event?: MouseEvent;
}

/**
 * A right-click (or the context-menu key) on a chart: on a data point (its series, x and y), or elsewhere on the plot
 * (the x under the pointer; no series or value). `x` and `y` are client coordinates for the menu.
 */
export interface ChartPointMenu {
  x: number;
  y: number;
  /** The x-axis value (epoch ms on Debrief's time charts). */
  at: number;
  series: string | null;
  value: number | null;
}

/** `[x, y]` of a series item's value (line and bar data are `[ts, value]` pairs); null when it is not one. */
export function pointOf(value: unknown): { at: number; value: number | null } | null {
  if (!Array.isArray(value) || typeof value[0] !== 'number' || !Number.isFinite(value[0])) return null;
  const y: unknown = value[1];
  return { at: value[0], value: typeof y === 'number' && Number.isFinite(y) ? y : null };
}

/** Thin ECharts host: owns the instance, resizes with its box, reports x-range brushes and right-clicks on the plot. */
@Component({
  selector: 'debrief-echart',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: '',
  styles: `:host { display: block; position: relative; min-height: 120px; }`,
})
export class EChart {
  readonly options = input.required<EChartsCoreOption>();
  /**
   * Merge updates into the existing chart (`notMerge: false`, series replaced by name) instead of rebuilding it:
   * live charts update in place without flicker.
   */
  readonly merge = input(false);
  /** Emits [from, to] (x-axis values) after the user drags across the plot. */
  readonly brushed = output<[number, number]>();
  /** A right-click on a data point or the plot (the browser's own menu is suppressed); see `ChartPointMenu`. */
  readonly pointMenu = output<ChartPointMenu>();
  /** A left click on the plot: the x-axis value under the pointer (epoch ms on time charts). */
  readonly plotClick = output<number>();

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly init = inject(ECHARTS_INIT);
  private chart: ECharts | null = null;
  private lastOptions: EChartsCoreOption | null = null;
  private lastMerge = false;
  private spreadTimer: ReturnType<typeof setTimeout> | undefined;
  /** The offsets last applied to the end labels; a re-render that computes the same ones is left alone (no loop). */
  private labelSig = '';

  constructor() {
    const destroyRef = inject(DestroyRef);

    effect(() => {
      const options = this.options();
      const merge = this.merge();
      if (this.chart) this.apply(options, merge);
    });

    afterNextRender(() => {
      const chart = this.init(this.host.nativeElement, undefined, { renderer: 'canvas' });
      this.chart = chart;
      chart.on('brushEnd', (e) => {
        const range = (e as BrushEndEvent).areas?.[0]?.coordRange;
        chart.dispatchAction({ type: 'brush', areas: [] });
        if (range && range[1] - range[0] > 0) this.brushed.emit([Math.floor(range[0]), Math.ceil(range[1])]);
      });
      // A right-click on a point names it. zrender reports every right-click, after the item's: the same click is
      // taken once (by its native event); one on the empty plot gives the x under the pointer.
      let itemClick: MouseEvent | null = null;
      chart.on('contextmenu', (e) => {
        const p = e as ItemMenuEvent;
        const point = p.componentType === 'series' ? pointOf(p.value) : null;
        const native = p.event?.event;
        if (!point || !native) return;
        itemClick = native;
        native.preventDefault();
        this.pointMenu.emit({ x: native.clientX, y: native.clientY, at: point.at, series: p.seriesName ?? null, value: point.value });
      });
      chart.getZr().on('contextmenu', (e) => {
        const z = e as unknown as ZrMenuEvent;
        if (z.event && z.event === itemClick) return;
        const pixel = [z.offsetX, z.offsetY];
        if (!z.event || !chart.containPixel({ gridIndex: 0 }, pixel)) return;
        const at: unknown = (chart.convertFromPixel({ gridIndex: 0 }, pixel) as unknown as unknown[] | undefined)?.[0];
        if (typeof at !== 'number' || !Number.isFinite(at)) return;
        z.event.preventDefault();
        this.pointMenu.emit({ x: z.event.clientX, y: z.event.clientY, at: Math.round(at), series: null, value: null });
      });
      chart.getZr().on('click', (e) => {
        const z = e as unknown as ZrMenuEvent;
        const pixel = [z.offsetX, z.offsetY];
        if (!chart.containPixel({ gridIndex: 0 }, pixel)) return;
        const at: unknown = (chart.convertFromPixel({ gridIndex: 0 }, pixel) as unknown as unknown[] | undefined)?.[0];
        if (typeof at === 'number' && Number.isFinite(at)) this.plotClick.emit(Math.round(at));
      });
      // ECharts does not lay end labels out against each other (labelLayout does not reach them): spread them after each render.
      // ECharts refuses setOption while it is still rendering ("main process"), so the re-apply runs on the next task.
      chart.on('finished', () => {
        if (this.spreadTimer !== undefined) return;
        this.spreadTimer = setTimeout(() => {
          this.spreadTimer = undefined;
          this.spreadEndLabels();
        });
      });
      const ro = new ResizeObserver(() => chart.resize());
      ro.observe(this.host.nativeElement);
      destroyRef.onDestroy(() => {
        ro.disconnect();
        clearTimeout(this.spreadTimer);
        chart.dispose();
        this.chart = null;
      });
      this.apply(this.options(), false);
    });
  }

  private apply(options: EChartsCoreOption, merge: boolean, keepSig = false): void {
    const chart = this.chart;
    if (!chart) return;
    if (!keepSig) {
      this.lastOptions = options;
      this.lastMerge = merge;
      this.labelSig = '';
    }
    chart.setOption(options, merge ? { notMerge: false, replaceMerge: ['series'], lazyUpdate: true } : { notMerge: true });
    // Arm x-range brushing without a toolbox: drag anywhere on the plot to select a time range.
    chart.dispatchAction({ type: 'takeGlobalCursor', key: 'brush', brushOption: { brushType: 'lineX', brushMode: 'single' } });
  }

  /**
   * ECharts does not lay end labels out against each other (`labelLayout` does not reach them, `endLabel.offset` is
   * ignored). After a render, measure where each line ends and, when labels would overprint, spread them (see
   * `spreadLabels`) and re-apply the option with those labels drawn as graphic text at the spread positions.
   */
  private spreadEndLabels(): void {
    const chart = this.chart;
    const options = this.lastOptions as { series?: EndSeries[] } | null;
    const series = options?.series;
    if (!chart || !options || !series) return;
    const labelled: { index: number; x: number; y: number }[] = [];
    series.forEach((s, index) => {
      if (s.type !== 'line' || !s.endLabel?.show || !s.data) return;
      for (let k = s.data.length - 1; k >= 0; k--) {
        const p = pointOf(s.data[k]);
        if (!p || p.value === null) continue;
        const px = chart.convertToPixel({ seriesIndex: index }, [p.at, p.value]) as unknown as number[] | undefined;
        if (px && Number.isFinite(px[0]) && Number.isFinite(px[1])) labelled.push({ index, x: px[0] as number, y: px[1] as number });
        break;
      }
    });
    if (labelled.length < 2) return;
    const offsets = spreadLabels(labelled.map((l) => l.y), undefined, 8, chart.getHeight() - 8).map((o) => Math.round(o));
    if (offsets.every((o) => o === 0)) return;
    const sig = labelled.map((l, i) => `${Math.round(l.x)}:${Math.round(l.y)}:${offsets[i]}`).join(',');
    if (sig === this.labelSig) return;
    this.labelSig = sig;
    // Overlapping end labels are drawn as graphic text at their spread positions instead of by the series.
    const patched = series.map((s) => s);
    const elements: object[] = [];
    labelled.forEach((l, i) => {
      const s = series[l.index];
      const e = s?.endLabel;
      if (!s || !e) return;
      patched[l.index] = { ...s, endLabel: { ...e, show: false } };
      elements.push({
        type: 'text',
        silent: true,
        x: l.x + (e.distance ?? 8),
        y: l.y + (offsets[i] ?? 0),
        style: { text: !e.formatter || e.formatter === '{a}' ? s.name : e.formatter, fill: e.color, font: `${e.fontSize ?? 10.5}px ${e.fontFamily ?? 'monospace'}`, textVerticalAlign: 'middle', verticalAlign: 'middle' },
      });
    });
    this.apply({ ...options, series: patched, graphic: { elements } } as EChartsCoreOption, this.lastMerge, true);
  }
}

interface EndSeries {
  name?: string;
  type?: string;
  data?: unknown[];
  endLabel?: { show?: boolean; distance?: number; formatter?: string; color?: string; fontSize?: number; fontFamily?: string };
}
