import { afterNextRender, ChangeDetectionStrategy, Component, DestroyRef, effect, ElementRef, inject, input, output } from '@angular/core';
import { BarChart, LineChart } from 'echarts/charts';
import { BrushComponent, DataZoomComponent, GridComponent, LegendComponent, MarkLineComponent, TooltipComponent } from 'echarts/components';
import { ECharts, EChartsCoreOption, init, use } from 'echarts/core';
import { CanvasRenderer } from 'echarts/renderers';

use([LineChart, BarChart, GridComponent, TooltipComponent, LegendComponent, DataZoomComponent, MarkLineComponent, BrushComponent, CanvasRenderer]);

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
  /** Emits [from, to] (x-axis values) after the user drags across the plot. */
  readonly brushed = output<[number, number]>();

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private chart: ECharts | null = null;

  constructor() {
    const destroyRef = inject(DestroyRef);

    effect(() => {
      const options = this.options();
      if (this.chart) this.apply(options);
    });

    afterNextRender(() => {
      const chart = init(this.host.nativeElement, undefined, { renderer: 'canvas' });
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
      this.apply(this.options());
    });
  }

  private apply(options: EChartsCoreOption): void {
    const chart = this.chart;
    if (!chart) return;
    chart.setOption(options, { notMerge: true });
    // Arm x-range brushing without a toolbox: drag anywhere on the plot to select a time range.
    chart.dispatchAction({ type: 'takeGlobalCursor', key: 'brush', brushOption: { brushType: 'lineX', brushMode: 'single' } });
  }
}
