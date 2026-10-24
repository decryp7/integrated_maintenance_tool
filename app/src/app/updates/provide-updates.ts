import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { provideNotificationSource } from '../health/provide-notification-host';
import { DEBRIEF_COMMANDS } from '../extensions/tokens';
import { createUpdatesBackend, UpdatesBackend } from './updates-backend';
import { UpdatesCommands } from './updates.commands';
import { UpdatesSource } from './updates-source';

/** Signed updates: the backend, Help ▸ Check for updates… and the update-available notification. One line in `app.config.ts`. */
export function provideUpdates(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: UpdatesBackend, useFactory: createUpdatesBackend },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(UpdatesCommands).commands },
    provideNotificationSource(UpdatesSource),
  ]);
}
