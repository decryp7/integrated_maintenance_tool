import { EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { provideToolsBackend } from '../core/tools-backend';
import { IMT_ACTIVITY_ITEMS, IMT_COMMANDS, IMT_VIEWS } from '../extensions/tokens';
import { CoreUiState } from '../lifecycle/core-ui-state';
import { LifecycleCommands } from '../lifecycle/lifecycle-commands';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { SessionService } from '../lifecycle/session.service';
import { UI_STATE_CONTRIBUTIONS } from '../lifecycle/ui-state';
import { BookmarkCommands } from '../projects/bookmark-commands';
import { OpenRequests } from '../projects/open-requests';
import { ProjectCommands } from '../projects/project-commands';
import { ProjectUiState } from '../projects/project-ui-state';
import { ToolCatalog } from './tool-catalog.store';
import { toolsActivityItems } from './tools-activity';
import { ToolsCommands, toolsViews } from './tools-commands';

/**
 * Registers the tools (Performance Monitor, process monitor, diagnostics collector, plugin
 * health), app lifecycle (administrator badge, safe mode and crash banners, session autosave),
 * projects and bookmarks. One line in `app.config.ts`.
 */
export function provideTools(): EnvironmentProviders {
  return makeEnvironmentProviders([
    ...provideToolsBackend(),
    { provide: IMT_VIEWS, multi: true, useFactory: () => toolsViews(inject(ToolCatalog)) },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(ToolsCommands).commands },
    { provide: IMT_ACTIVITY_ITEMS, multi: true, useFactory: toolsActivityItems },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(LifecycleCommands).commands },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(ProjectCommands).commands },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(BookmarkCommands).commands },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => inject(CoreUiState).contributions },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => inject(ProjectUiState).contributions },
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
  void life.init().then(() => {
    session.start();
    return requests.start();
  });
}
