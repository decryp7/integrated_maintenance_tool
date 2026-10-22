import { InjectionToken } from '@angular/core';
import { BarChart, LineChart } from 'echarts/charts';
import { BrushComponent, DataZoomComponent, GridComponent, LegendComponent, MarkAreaComponent, MarkLineComponent, ToolboxComponent, TooltipComponent } from 'echarts/components';
import { ECharts, init, use } from 'echarts/core';
import { CanvasRenderer } from 'echarts/renderers';

// Brushing always adds a toolbox to the option (ECharts' brush preprocessor), so it must be registered; charts hide it.
use([LineChart, BarChart, GridComponent, TooltipComponent, LegendComponent, DataZoomComponent, MarkLineComponent, MarkAreaComponent, BrushComponent, ToolboxComponent, CanvasRenderer]);

/** Creates a chart instance on an element (ECharts' `init`, with the chart types IMT uses registered). */
export type EChartsInit = (host: HTMLElement, theme: undefined, opts: { renderer: 'canvas' }) => ECharts;

/**
 * How `EChart` creates its instance. Defaults to ECharts itself; tests provide a fake through DI instead of
 * `vi.mock('echarts/core')`, which is unreliable when spec files share one module graph (Vitest `isolate: false`).
 */
export const ECHARTS_INIT = new InjectionToken<EChartsInit>('ECHARTS_INIT', { providedIn: 'root', factory: () => init });
