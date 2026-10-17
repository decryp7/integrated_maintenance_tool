import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { AnalysisBackend, TauriAnalysisBackend } from '../core/analysis-backend';
import { ImtBackend } from '../core/backend';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { IMT_COMMANDS, ImtView } from '../extensions/tokens';
import { provideImtPlugin } from '../extensions/provide';
import { AnalysisCommands } from './analysis-commands';
import { MockAnalysisBackend, recordsFrom } from './mock/mock-analysis-backend';

/** The analysis views. Only the query bar opens by default (it follows the workspace); the rest open from the palette. */
export const ANALYSIS_VIEWS: ImtView[] = [
  {
    id: 'query',
    title: 'Query',
    icon: 'search',
    defaultPosition: 'bottom',
    component: () => import('./query-view').then((m) => m.QueryView),
    when: (s) => s.total > 0,
  },
  { id: 'query-results', title: 'Query results', icon: 'columns', defaultPosition: 'bottom', component: () => import('./query-results-view').then((m) => m.QueryResultsView) },
  { id: 'moment', title: 'Moment', icon: 'timeline', defaultPosition: 'center', component: () => import('./moment-view').then((m) => m.MomentView) },
  { id: 'anomalies', title: 'Anomalies', icon: 'alert', defaultPosition: 'bottom', component: () => import('./anomalies-view').then((m) => m.AnomaliesView) },
  { id: 'patterns', title: 'Patterns', icon: 'group', defaultPosition: 'bottom', component: () => import('./patterns-view').then((m) => m.PatternsView) },
  { id: 'exceptions', title: 'Exceptions', icon: 'alert', defaultPosition: 'bottom', component: () => import('./exceptions-view').then((m) => m.ExceptionsView) },
  { id: 'compare', title: 'Compare', icon: 'columns', defaultPosition: 'center', component: () => import('./compare-view').then((m) => m.CompareView) },
  { id: 'field-stats', title: 'Field statistics', icon: 'chart', defaultPosition: 'center', component: () => import('./field-stats-view').then((m) => m.FieldStatsView) },
  { id: 'traces', title: 'Traces', icon: 'timeline', defaultPosition: 'center', component: () => import('./traces-view').then((m) => m.TracesView) },
];

/** The engine-backed analysis backend inside Tauri, the mock (derived from the demo records) in a browser. */
export function createAnalysisBackend(): AnalysisBackend {
  const backend: AnalysisBackend = isTauri() ? new TauriAnalysisBackend() : new MockAnalysisBackend(recordsFrom(inject(ImtBackend)));
  return withFailureLogging<AnalysisBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}

/** One line in `app.config.ts`: the analysis backend, views and commands. */
export function provideAnalysis(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: AnalysisBackend, useFactory: createAnalysisBackend },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(AnalysisCommands).commands },
    provideImtPlugin({ views: ANALYSIS_VIEWS }),
  ]);
}
