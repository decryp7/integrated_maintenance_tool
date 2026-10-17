import { EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { Logger } from '../core/logger';
import { POPOUT_ROUTE } from '../popout/popout-route';
import { IMT_COMMANDS } from '../extensions/tokens';
import { LocalStorageRecentProjects, RECENT_PROJECTS } from '../projects/recent-projects';
import { TrayRecentProjects } from './tray-recent-projects';
import { WindowCommands } from './window-commands';
import { WindowTitleSync } from './window-title';
import { provideWindowingBackend, WindowingBackend } from './windowing-backend';
import { WindowingService } from './windowing.service';

/**
 * Multiple windows, the system tray and background work: window commands, the recent-projects
 * feed of the tray menu, the window title and the reactions to the shell's window events.
 * One line in `app.config.ts`.
 */
export function provideWindowing(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideWindowingBackend(),
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(WindowCommands).commands },
    { provide: RECENT_PROJECTS, useFactory: () => new TrayRecentProjects(new LocalStorageRecentProjects(), inject(WindowingBackend), inject(Logger)) },
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
