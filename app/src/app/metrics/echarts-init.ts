import { InjectionToken } from '@angular/core';
import { BarChart, LineChart } from 'echarts/charts';
import { BrushComponent, DataZoomComponent, GridComponent, LegendComponent, MarkLineComponent, TooltipComponent } from 'echarts/components';
import { ECharts, init, use } from 'echarts/core';
import { CanvasRenderer } from 'echarts/renderers';

use([LineChart, BarChart, GridComponent, TooltipComponent, LegendComponent, DataZoomComponent, MarkLineComponent, BrushComponent, CanvasRenderer]);

/** Creates a chart instance on an element (ECharts' `init`, with the chart types IMT uses registered). */
export type EChartsInit = (host: HTMLElement, theme: undefined, opts: { renderer: 'canvas' }) => ECharts;

/**
 * How `EChart` creates its instance. Defaults to ECharts itself; tests provide a fake through DI instead of
 * `vi.mock('echarts/core')`, which is unreliable when spec files share one module graph (Vitest `isolate: false`).
 */
export const ECHARTS_INIT = new InjectionToken<EChartsInit>('ECHARTS_INIT', { providedIn: 'root', factory: () => init });
