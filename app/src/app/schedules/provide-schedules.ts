import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';

/** The scheduled-collection views. Same extension point as plugin views. */
export const SCHEDULES_VIEWS: ImtView[] = [
  { id: 'collection-schedules', category: 'Tools', title: 'Collection schedules', icon: 'timeline', defaultPosition: 'center', component: () => import('./schedules-view').then((m) => m.SchedulesView) },
  { id: 'collection-runs', category: 'Tools', title: 'Collection runs', icon: 'folder', defaultPosition: 'center', component: () => import('./runs-view').then((m) => m.RunsView) },
];

export const SCHEDULES_COMMANDS: ImtCommand[] = [
  { id: 'tools.schedules.show', title: 'Open collection schedules', category: 'Tools', run: (ctx) => ctx.showView('collection-schedules') },
  { id: 'tools.runs.show', title: 'Open collection runs', category: 'Tools', run: (ctx) => ctx.showView('collection-runs') },
];

/**
 * Scheduled collection with the headless agent (`imt-agent`): the schedules editor and the runs
 * browser. One line in `app.config.ts`.
 */
export function provideSchedules(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_VIEWS, multi: true, useValue: SCHEDULES_VIEWS },
    { provide: IMT_COMMANDS, multi: true, useValue: SCHEDULES_COMMANDS },
  ]);
}
