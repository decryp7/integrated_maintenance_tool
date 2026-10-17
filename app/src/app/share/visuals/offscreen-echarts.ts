import type { ECharts } from 'echarts/core';
import { init, use } from 'echarts/core';
import { SVGRenderer } from 'echarts/renderers';
import { ECHARTS_INIT } from '../../metrics/echarts-init';
import type { OffscreenOpts } from './visual-export.service';

// Loaded on demand (not part of the initial bundle): the chart types are registered by echarts-init;
// image exports also need the SVG renderer.
use([SVGRenderer]);
void ECHARTS_INIT;

/** ECharts' `init` for off-screen export instances. */
export const offscreenInit = init as unknown as (host: HTMLElement | null, theme: undefined, opts: OffscreenOpts) => ECharts;
