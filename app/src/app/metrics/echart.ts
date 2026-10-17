import { afterNextRender, ChangeDetectionStrategy, Component, DestroyRef, effect, ElementRef, inject, input, output } from '@angular/core';
import type { ECharts, EChartsCoreOption } from 'echarts/core';
import { ECHARTS_INIT } from './echarts-init';

interface BrushEndEvent {
  areas?: { coordRange?: [number, number] }[];
}

/** Thin ECharts host: owns the instance, resizes with its box, reports x-range brushes. */
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
