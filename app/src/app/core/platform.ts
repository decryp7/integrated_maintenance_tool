import { Injectable } from '@angular/core';

/** Host integration: native dialogs and drag-and-drop when running inside Tauri. */

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** Shows the native open dialog. Returns the chosen paths (empty when cancelled). */
export async function pickPaths(directory: boolean): Promise<string[]> {
  if (!isTauri()) {
    // Browser development mode: the mock backend understands any path.
    return [directory ? '/demo/extra-folder' : '/demo/extra-service.log'];
  }
  const { open } = await import('@tauri-apps/plugin-dialog');
  const result = await open({
    multiple: true,
    directory,
    title: directory ? 'Open folder' : 'Open log files',
  });
  if (result === null) return [];
  return Array.isArray(result) ? result : [result];
}

/** Subscribes to files dropped on the window. Returns an unsubscribe function. */
export async function onFilesDropped(
  handler: (paths: string[]) => void,
  hover: (over: boolean) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { getCurrentWebview } = await import('@tauri-apps/api/webview');
  return getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    if (payload.type === 'enter' || payload.type === 'over') hover(true);
    else if (payload.type === 'leave') hover(false);
    else if (payload.type === 'drop') {
      hover(false);
      if (payload.paths.length) handler(payload.paths);
    }
  });
}

/** Native open dialog for a single profile file; null when cancelled. */
export async function pickProfileToOpen(): Promise<string | null> {
  if (!isTauri()) return '/demo/profiles/imported.toml';
  const { open } = await import('@tauri-apps/plugin-dialog');
  const result = await open({ multiple: false, directory: false, title: 'Import profile', filters: [{ name: 'IMT profile', extensions: ['toml'] }] });
  return typeof result === 'string' ? result : null;
}

/** Native save dialog for a profile file; null when cancelled. */
export async function pickProfileToSave(defaultName: string): Promise<string | null> {
  if (!isTauri()) return defaultName;
  const { save } = await import('@tauri-apps/plugin-dialog');
  return save({ title: 'Export profile', defaultPath: defaultName, filters: [{ name: 'IMT profile', extensions: ['toml'] }] });
}

/** Injectable facade over the host functions, so components and the store can be tested with fakes. */
@Injectable({ providedIn: 'root' })
export class Platform {
  readonly isTauri = isTauri;
  pickPaths(directory: boolean): Promise<string[]> {
    return pickPaths(directory);
  }
  onFilesDropped(handler: (paths: string[]) => void, hover: (over: boolean) => void): Promise<() => void> {
    return onFilesDropped(handler, hover);
  }
  pickProfileToOpen(): Promise<string | null> {
    return pickProfileToOpen();
  }
  pickProfileToSave(defaultName: string): Promise<string | null> {
    return pickProfileToSave(defaultName);
  }
}
