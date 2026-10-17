import { InjectionToken } from '@angular/core';

/**
 * A feature's claim on files dropped on the window: it opens the paths it handles and resolves to
 * those paths. Whatever no handler claims is opened as logs, so no file is imported twice.
 */
export type DroppedFilesHandler = (paths: string[]) => Promise<readonly string[]>;

/** Multi token: features (e.g. profiling) contribute handlers; the workspace routes every drop through them. */
export const DROPPED_FILES_HANDLERS = new InjectionToken<readonly DroppedFilesHandler[]>('DROPPED_FILES_HANDLERS');

/**
 * Offers the dropped paths to each handler in turn and resolves to the paths none claimed. A
 * handler that fails claims nothing (it reports its own failures), so the files still open as logs.
 */
export async function routeDroppedFiles(handlers: readonly DroppedFilesHandler[], paths: readonly string[]): Promise<string[]> {
  let rest = [...paths];
  for (const handler of handlers) {
    if (!rest.length) break;
    try {
      const claimed = new Set(await handler(rest));
      rest = rest.filter((p) => !claimed.has(p));
    } catch {
      // Nothing claimed.
    }
  }
  return rest;
}
