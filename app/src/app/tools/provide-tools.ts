import { WhatsNewService } from '../whats-new/whats-new';
import { EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { DROPPED_FILES_HANDLERS } from '../core/dropped-files';
import { provideToolsBackend } from '../core/tools-backend';
import { DEBRIEF_ACTIVITY_ITEMS, DEBRIEF_COMMANDS, DEBRIEF_VIEWS } from '../extensions/tokens';
import { CoreUiState } from '../lifecycle/core-ui-state';
import { LifecycleCommands } from '../lifecycle/lifecycle-commands';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { SessionService } from '../lifecycle/session.service';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { OpenRequests } from '../projects/open-requests';
import { ProjectCommands } from '../projects/project-commands';
import { ProjectUiState } from '../projects/project-ui-state';
import { SavedQueryCommands } from '../projects/saved-query-commands';
import { pluginPackageDropHandler } from '../plugins-panel/package-drop';
import { ToolCatalog } from './tool-catalog.store';
import { toolsActivityItems } from './tools-activity';
import { ToolsCommands, toolsViews } from './tools-commands';

/**
 * Registers the tools (Performance Monitor, process monitor, diagnostics collector, plugin
 * health), app lifecycle (administrator badge, safe mode and crash banners, session autosave),
 * projects and saved queries. One line in `app.config.ts`.
 */
export function provideTools(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideToolsBackend(),
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: () => toolsViews(inject(ToolCatalog)) },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(ToolsCommands).commands },
    { provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: toolsActivityItems },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(LifecycleCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(ProjectCommands).commands },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => inject(SavedQueryCommands).commands },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => inject(CoreUiState).contributions },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => inject(ProjectUiState).contributions },
    // `.debriefplugin` files dropped on the window are reviewed for installing, never opened as logs.
    { provide: DROPPED_FILES_HANDLERS, multi: true, useFactory: pluginPackageDropHandler },
    provideAppInitializer(() => startTools()),
  ]);
}

/** Starts the background parts. Not awaited: a slow backend must not delay the first paint. */
export function startTools(): void {
  const life = inject(LifecycleService);
  const session = inject(SessionService);
  const requests = inject(OpenRequests);
  // Reads which tools the engine offers (and keeps it current); views and commands follow.
  inject(ToolCatalog);
  const whatsNew = inject(WhatsNewService);
  void life.init().then(() => {
    whatsNew.checkOnStart();
    session.start();
    return requests.start();
  });
}
