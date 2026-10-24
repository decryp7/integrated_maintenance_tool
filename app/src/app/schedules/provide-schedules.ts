import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';

/** The scheduled-collection views. Same extension point as plugin views. */
export const SCHEDULES_VIEWS: DebriefView[] = [
  { id: 'collection-schedules', category: 'Tools', title: $localize`:@@schedules.view.schedules:Collection schedules`, icon: 'timeline', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./schedules-view').then((m) => m.SchedulesView) },
  { id: 'collection-runs', category: 'Tools', title: $localize`:@@schedules.view.runs:Collection runs`, icon: 'folder', defaultPosition: 'center', layout: { area: 'status', min: { width: 320, height: 180 }, pref: { width: 480, height: 360 }, priority: 20 }, component: () => import('./runs-view').then((m) => m.RunsView) },
];

export const SCHEDULES_COMMANDS: DebriefCommand[] = [
  { id: 'tools.schedules.show', title: $localize`:@@schedules.cmd.schedules:Open collection schedules`, category: $localize`:@@schedules.category.tools:Tools`, run: (ctx) => ctx.showView('collection-schedules') },
  { id: 'tools.runs.show', title: $localize`:@@schedules.cmd.runs:Open collection runs`, category: $localize`:@@schedules.category.tools:Tools`, run: (ctx) => ctx.showView('collection-runs') },
];

/**
 * Scheduled collection with the headless agent (`debrief-agent`): the schedules editor and the runs
 * browser. One line in `app.config.ts`.
 */
export function provideSchedules(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useValue: SCHEDULES_VIEWS },
    { provide: DEBRIEF_COMMANDS, multi: true, useValue: SCHEDULES_COMMANDS },
  ]);
}
