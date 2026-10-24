import { effect, EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideEnvironmentInitializer, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { provideDebriefPlugin } from '../extensions/provide';
import { DebriefView } from '../extensions/tokens';
import { ProjectService } from '../projects/project.service';
import { HomePrefs } from './home-prefs';

export const HOME_VIEW = 'home';

export const HOME_VIEWS: DebriefView[] = [
  {
    id: HOME_VIEW,
    title: $localize`:@@home.plugin.title:Home`,
    category: 'core',
    icon: 'home',
    // The app's home screen and dashboard: a document beside the timeline (a right-hand tool window is too cramped).
    defaultPosition: 'center',
    layout: { area: 'document', min: { width: 420, height: 300 }, pref: { width: 700, height: 600 }, priority: 60 },
    fullHeight: true,
    component: () => import('./home-view').then((m) => m.HomeView),
  },
];

/**
 * Home is the home screen: it opens as a document tab after every successful open of files, folders or a project,
 * unless the user turned that off (Settings ▸ General). The health headline in the timeline header says the same in
 * one line (`HealthHeadline`).
 */
@Injectable({ providedIn: 'root' })
export class HomeAutoOpen {
  private readonly store = inject(WorkspaceStore);
  private readonly dock = inject(DockService);
  private readonly prefs = inject(HomePrefs);
  private readonly projects = inject(ProjectService);

  constructor() {
    let seenProject = this.projects.openedCount();
    effect(() => {
      const count = this.projects.openedCount();
      if (count === seenProject) return;
      seenProject = count;
      untracked(() => {
        if (this.prefs.autoOpen()) this.dock.show(HOME_VIEW);
      });
    });
    let seen = this.store.importVersion();
    effect(() => {
      const version = this.store.importVersion();
      if (version === seen) return;
      seen = version;
      untracked(() => {
        if (this.prefs.autoOpen()) this.dock.show(HOME_VIEW);
      });
    });
  }
}

/** One line in `app.config.ts`: the Home view, its command and opening it after an import. */
export function provideHome(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideDebriefPlugin({
      views: HOME_VIEWS,
      commands: [{ id: 'view.home', title: $localize`:@@home.plugin.show:Show Home`, category: $localize`:@@home.plugin.category:View`, run: (ctx) => ctx.showView(HOME_VIEW) }],
    }),
    provideEnvironmentInitializer(() => inject(HomeAutoOpen)),
  ]);
}
