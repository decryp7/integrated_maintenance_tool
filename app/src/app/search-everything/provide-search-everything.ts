import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { DEBRIEF_COMMANDS, DebriefCommand } from '../extensions/tokens';
import { SearchEverythingLauncher } from './search-everything-launcher';

/** The command that opens the search everything palette. */
export const SEARCH_EVERYTHING_COMMAND = 'search.everything';
export const SEARCH_EVERYTHING_KEYBINDING = 'Mod+Shift+F';

export function searchEverythingCommands(launcher: SearchEverythingLauncher): DebriefCommand[] {
  return [
    {
      id: SEARCH_EVERYTHING_COMMAND,
      title: $localize`:@@search.cmd.open:Search everything…`,
      category: $localize`:Command palette category|@@search.category:Search`,
      keybinding: SEARCH_EVERYTHING_KEYBINDING,
      run: () => launcher.open(),
    },
  ];
}

/** One line in `app.config.ts`: the search everything palette and its command (Mod+Shift+F). */
export function provideSearchEverything(): EnvironmentProviders {
  return makeEnvironmentProviders([{ provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => searchEverythingCommands(inject(SearchEverythingLauncher)) }]);
}
