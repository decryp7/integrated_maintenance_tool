import { effect, EnvironmentProviders, inject, Injector, makeEnvironmentProviders, provideAppInitializer } from '@angular/core';
import { DROPPED_FILES_HANDLERS, routeDroppedFiles } from '../dropped-files';
import { Platform } from '../platform';
import { ShellUiStore } from './shell-ui.store';
import { ThemeService } from './theme.service';
import { savePrefs, UiPrefsStore } from './ui-prefs.store';
import { WorkspaceDataStore } from './workspace-data.store';

/** Saves the view preferences whenever they change. */
export function persistPrefs(injector: Injector): void {
  const prefs = injector.get(UiPrefsStore);
  effect(() => savePrefs(prefs.snapshot()), { injector });
}

/**
 * Routes files dropped on the window: features claim their files first (profiling: captures); the rest open as
 * logs, once. The drop overlay follows the drag.
 */
export function routeDrops(injector: Injector): void {
  const handlers = injector.get(DROPPED_FILES_HANDLERS, null, { optional: true }) ?? [];
  const shell = injector.get(ShellUiStore);
  void injector.get(Platform).onFilesDropped(
    (paths) => void routeDroppedFiles(handlers, paths).then((rest) => (rest.length ? injector.get(WorkspaceDataStore).openPaths(rest) : undefined)),
    (over) => shell.setDropHover(over),
  );
}

/**
 * The window's workspace side effects at start-up: theme, preference persistence and file drops. The first summary
 * loads when something first uses the workspace data (`WorkspaceDataStore`).
 */
export function startWorkspace(): void {
  const injector = inject(Injector);
  injector.get(ThemeService).start();
  persistPrefs(injector);
  routeDrops(injector);
}

/** Starts the workspace with the app (main window and pop-outs). */
export function provideWorkspace(): EnvironmentProviders {
  return makeEnvironmentProviders([provideAppInitializer(startWorkspace)]);
}
