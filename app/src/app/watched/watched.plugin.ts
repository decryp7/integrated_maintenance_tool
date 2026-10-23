import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { provideDebriefPlugin } from '../extensions/provide';
import { DebriefCommand, DebriefView } from '../extensions/tokens';

const CATEGORY = $localize`:@@watched.commands.category:Workspace`;

export const WATCHED_VIEWS: DebriefView[] = [
  {
    id: 'watched-folders',
    category: 'Tools',
    title: $localize`:@@watched.view.title:Watched folders`,
    icon: 'folder',
    defaultPosition: 'bottom',
    component: () => import('./watched-folders-view').then((m) => m.WatchedFoldersView),
  },
];

export const WATCHED_COMMANDS: DebriefCommand[] = [
  { id: 'watched.show', title: $localize`:@@watched.commands.show:Show watched folders`, category: CATEGORY, run: (ctx) => ctx.showView('watched-folders') },
];

/** Watched folders: the view and its command. */
export function provideWatched(): EnvironmentProviders {
  return makeEnvironmentProviders([provideDebriefPlugin({ views: WATCHED_VIEWS, commands: WATCHED_COMMANDS })]);
}
