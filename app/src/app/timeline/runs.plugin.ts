import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { provideImtPlugin } from '../extensions/provide';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';

/** Palette commands of runs between lifecycle markers. */
export const RUNS_COMMANDS: ImtCommand[] = [
  { id: 'runs.show', title: $localize`:@@runs.cmd.show.title:Show runs`, category: $localize`:Command palette category|@@runs.cmd.category:Runs`, run: (ctx) => ctx.showView('runs') },
  { id: 'runs.setUp', title: $localize`:@@runs.cmd.setUp.title:Set up runs between start and stop markers…`, category: $localize`:Command palette category|@@runs.cmd.category:Runs`, run: (ctx) => ctx.showView('markers') },
];

/** Registers the Runs view and its commands. One line in app.config.ts: `provideRuns()`. */
export function provideRuns(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideImtPlugin({
      views: [{ id: 'runs', category: 'Analysis', title: $localize`:View title|@@runs.view.viewTitle:Runs`, icon: 'timeline', defaultPosition: 'bottom', component: () => import('./runs-view').then((m) => m.RunsView) }],
    }),
    { provide: IMT_COMMANDS, multi: true, useValue: RUNS_COMMANDS },
  ]);
}
