import { ApplicationRef, createComponent, EnvironmentInjector, EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { createUpdatesBackend, UpdatesBackend } from './updates-backend';
import { UpdatesCommands } from './updates.commands';
import { UpdatesToast } from './updates-toast';

/** Mounts the "update available" toast once over the app. */
export function startUpdates(): void {
  const host = document.createElement('debrief-updates-toast');
  document.body.append(host);
  const ref = createComponent(UpdatesToast, { environmentInjector: inject(EnvironmentInjector), hostElement: host });
  inject(ApplicationRef).attachView(ref.hostView);
}

/** Signed updates: the backend, Help ▸ Check for updates… and the toast. One line in `app.config.ts`. */
export function provideUpdates(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: UpdatesBackend, useFactory: createUpdatesBackend },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(UpdatesCommands).commands },
    provideAppInitializer(() => startUpdates()),
  ]);
}
