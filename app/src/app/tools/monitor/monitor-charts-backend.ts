import { inject, InjectionToken } from '@angular/core';
import { ChartData, ChartRequest } from '../../core/api.types';
import { ImtBackend } from '../../core/backend';
import { isTauri } from '../../core/platform';
import { CollectBackend } from '../../core/tools-backend';
import type { MockToolsBackend } from '../mock/mock-tools-backend';
import { browserMocks } from '../../core/mock-registry';

/** The one engine operation the live panel needs: a chart query (`query_chart`) over the monitor's events. */
export interface MonitorCharts {
  queryChart(request: ChartRequest): Promise<ChartData>;
}

/**
 * The engine's chart query inside Tauri. In a plain browser (ng serve, e2e) the mock workspace holds no monitor
 * samples, so a mock that answers the same contract with plausible "Process stats" series stands in.
 */
export const MONITOR_CHARTS = new InjectionToken<MonitorCharts>('MONITOR_CHARTS', {
  providedIn: 'root',
  factory: () => {
    if (isTauri()) {
      const backend = inject(ImtBackend);
      return { queryChart: (r: ChartRequest) => backend.queryChart(r) };
    }
    // The mock tools backend remembers the run's config: .NET samples appear only when the run asked for them.
    const tools = inject(CollectBackend, { optional: true }) as Partial<Pick<MockToolsBackend, 'monitorConfig'>> | null;
    const options = browserMocks().mockMonitorChartsOptions(typeof location === 'undefined' ? '' : location.search);
    return new (browserMocks().MockMonitorCharts)({ ...options, dotnet: () => tools?.monitorConfig?.dotnet === true });
  },
});
