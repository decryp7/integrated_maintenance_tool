import { EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { POPOUT_ROUTE } from '../popout/popout-route';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { WindowCommands } from './window-commands';
import { WindowTitleSync } from './window-title';
import { provideWindowingBackend } from './windowing-backend';
import { WindowingService } from './windowing.service';

/**
 * Multiple windows, the system tray and background work: window commands, the window title and the reactions to the shell's window events.
 * One line in `app.config.ts`.
 */
export function provideWindowing(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideWindowingBackend(),
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(WindowCommands).commands },
    provideAppInitializer(() => startWindowing()),
  ]);
}

/**
 * Starts listening to window events and syncing the title. Not awaited: the first paint must not wait. A pop-out
 * panel window does neither: its host owns the title, the close and open prompts and the window list.
 */
export function startWindowing(): void {
  if (inject(POPOUT_ROUTE)) return;
  void inject(WindowingService).init();
  inject(WindowTitleSync).start();
}
