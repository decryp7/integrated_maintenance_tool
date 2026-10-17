import { EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { Logger } from '../core/logger';
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

/** Starts listening to window events and syncing the title. Not awaited: the first paint must not wait. */
export function startWindowing(): void {
  void inject(WindowingService).init();
  inject(WindowTitleSync).start();
}
