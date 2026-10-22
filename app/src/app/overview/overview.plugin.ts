import { effect, EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideEnvironmentInitializer, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { provideImtPlugin } from '../extensions/provide';
import { ImtView } from '../extensions/tokens';
import { OverviewPrefs } from './overview-prefs';

export const OVERVIEW_VIEW = 'overview';

export const OVERVIEW_VIEWS: ImtView[] = [
  {
    id: OVERVIEW_VIEW,
    title: $localize`:@@overview.plugin.title:Overview`,
    category: 'core',
    icon: 'check',
    // Next to the inspector, so it never hides the timeline or the events.
    defaultPosition: 'right',
    component: () => import('./overview-view').then((m) => m.OverviewView),
  },
];

/**
 * In the Full layout, opens the overview after every successful open of files or folders, unless the user turned it
 * off. The Focused layout shows its headline in the timeline header instead (`OverviewHeadline`).
 */
@Injectable({ providedIn: 'root' })
export class OverviewAutoOpen {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);
  private readonly prefs = inject(OverviewPrefs);

  constructor() {
    let seen = this.store.importVersion();
    effect(() => {
      const version = this.store.importVersion();
      if (version === seen) return;
      seen = version;
      untracked(() => {
        if (this.prefs.autoOpen()) this.dock.show(OVERVIEW_VIEW);
      });
    });
  }
}

/** One line in `app.config.ts`: the Overview view, its command and opening it after an import. */
export function provideOverview(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideImtPlugin({
      views: OVERVIEW_VIEWS,
      commands: [{ id: 'view.overview', title: $localize`:@@overview.plugin.show:Show overview`, category: $localize`:@@overview.plugin.category:View`, run: (ctx) => ctx.showView(OVERVIEW_VIEW) }],
    }),
    provideEnvironmentInitializer(() => inject(OverviewAutoOpen)),
  ]);
}
