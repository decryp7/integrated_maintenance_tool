import { EnvironmentProviders, inject, makeEnvironmentProviders, untracked } from '@angular/core';
import { DEBRIEF_ACTIVITY_ITEMS, DEBRIEF_COMMANDS } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { DockCommands } from './dock-commands';
import { DockService } from './dock.service';
import { perspectiveActivityItems } from './perspective-activity';
import { Perspectives } from './perspectives';
import { readLayout, toEnvelope } from './layout-store';

/**
 * The whole dock arrangement in projects and sessions (grid, auto-hidden tool windows, pinned tabs and pop-out
 * windows with their bounds). Layout is not a project edit: it never marks a project unsaved.
 */
export function dockLayoutContribution(dock: DockService): UiStateContribution {
  return {
    key: 'dockLayout',
    tracksChanges: false,
    capture: () => {
      // Reactive on the persisted layout and the open panels; captures the arrangement as it is right now (a save
      // right after opening a panel must not miss it while the debounced persist is pending).
      dock.savedLayout();
      dock.openIds();
      const now = untracked(() => dock.currentLayout());
      return now ? toEnvelope(now) : null;
    },
    restore: (value) => {
      const saved = readLayout(value);
      if (saved) dock.applyLayout(saved);
    },
  };
}

/** Visual Studio-style docking: window commands (layouts, auto hide, full screen, ...) and layout persistence. One line in `app.config.ts`. */
export function provideDocking(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(DockCommands).commands },
    { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: () => perspectiveActivityItems(inject(Perspectives)) },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [dockLayoutContribution(inject(DockService))] },
  ]);
}
