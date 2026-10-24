import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { provideDebriefPlugin } from '../extensions/provide';
import { DEBRIEF_COMMANDS, DebriefCommand } from '../extensions/tokens';

/** Palette commands of runs between lifecycle markers. */
export const RUNS_COMMANDS: DebriefCommand[] = [
  { id: 'runs.show', title: $localize`:@@runs.cmd.show.title:Show runs`, category: $localize`:Command palette category|@@runs.cmd.category:Runs`, run: (ctx) => ctx.showView('runs') },
  { id: 'runs.setUp', title: $localize`:@@runs.cmd.setUp.title:Set up runs between start and stop markers…`, category: $localize`:Command palette category|@@runs.cmd.category:Runs`, run: (ctx) => ctx.showView('markers') },
];

/** Registers the Runs view and its commands. One line in app.config.ts: `provideRuns()`. */
export function provideRuns(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideDebriefPlugin({
      views: [{ id: 'runs', category: 'Analysis', title: $localize`:View title|@@runs.view.viewTitle:Runs`, icon: 'timeline', defaultPosition: 'bottom', layout: { area: 'analysis', min: { width: 420, height: 180 }, pref: { width: 560, height: 280 }, priority: 50 }, component: () => import('./runs-view').then((m) => m.RunsView) }],
    }),
    { provide: DEBRIEF_COMMANDS, multi: true, useValue: RUNS_COMMANDS },
  ]);
}
