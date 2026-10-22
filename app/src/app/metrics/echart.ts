import { afterNextRender, ChangeDetectionStrategy, Component, DestroyRef, effect, ElementRef, inject, input, output } from '@angular/core';
import type { ECharts, EChartsCoreOption } from 'echarts/core';
import { ECHARTS_INIT } from './echarts-init';

interface BrushEndEvent {
  areas?: { coordRange?: [number, number] }[];
}

/** What ECharts passes to a series item's `contextmenu` handler (the parts IMT reads). */
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
  /** The x-axis value (epoch ms on IMT's time charts). */
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
  selector: 'imt-echart',
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
      const ro = new ResizeObserver(() => chart.resize());
      ro.observe(this.host.nativeElement);
      destroyRef.onDestroy(() => {
        ro.disconnect();
        chart.dispose();
        this.chart = null;
      });
      this.apply(this.options(), false);
    });
  }

  private apply(options: EChartsCoreOption, merge: boolean): void {
    const chart = this.chart;
    if (!chart) return;
    chart.setOption(options, merge ? { notMerge: false, replaceMerge: ['series'], lazyUpdate: true } : { notMerge: true });
    // Arm x-range brushing without a toolbox: drag anywhere on the plot to select a time range.
    chart.dispatchAction({ type: 'takeGlobalCursor', key: 'brush', brushOption: { brushType: 'lineX', brushMode: 'single' } });
  }
}
