import { Injectable } from '@angular/core';
import { nativePick, nativePickOne } from './native-dialog';
import type { DropPoint } from './dropped-files';

/** Host integration: native dialogs and drag-and-drop when running inside Tauri. */

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** Shows the native open dialog. Returns the chosen paths (empty when cancelled). */
export async function pickPaths(directory: boolean): Promise<string[]> {
  if (!isTauri()) {
    // Browser development mode: the mock backend understands any path; `?mockOpen=<path>` picks that one (UI tests).
    const forced = typeof location === 'undefined' ? null : new URLSearchParams(location.search).get('mockOpen');
    if (forced) return [forced];
    return [directory ? '/demo/extra-folder' : '/demo/extra-service.log'];
  }
  return nativePick({
    kind: directory ? 'openFolders' : 'openFiles',
    title: directory ? $localize`:@@core.dialog.openFolder:Open folder` : $localize`:@@core.dialog.openLogFiles:Open log files`,
  });
}

/** Subscribes to files dropped on the window. Returns an unsubscribe function. */
export async function onFilesDropped(
  handler: (paths: string[], at: DropPoint | null) => void,
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
      if (payload.paths.length) handler(payload.paths, dropPoint(payload.position, window.devicePixelRatio));
    }
  });
}

/** The OS drop position (physical pixels) in the page's CSS pixels. */
export function dropPoint(position: { x: number; y: number } | null | undefined, scale: number): DropPoint | null {
  if (!position || !Number.isFinite(position.x) || !Number.isFinite(position.y)) return null;
  const s = Number.isFinite(scale) && scale > 0 ? scale : 1;
  return { x: position.x / s, y: position.y / s };
}

/** Native open dialog for a single profile file; null when cancelled. */
export async function pickProfileToOpen(): Promise<string | null> {
  if (!isTauri()) return '/demo/profiles/imported.toml';
  return nativePickOne({ kind: 'openFile', title: $localize`:@@core.dialog.importProfile:Import profile`, filters: [{ name: $localize`:@@core.dialog.profileFilter:Debrief profile`, extensions: ['toml'] }] });
}

/** Native save dialog for a profile file; null when cancelled. */
export async function pickProfileToSave(defaultName: string): Promise<string | null> {
  if (!isTauri()) return defaultName;
  return nativePickOne({ kind: 'save', title: $localize`:@@core.dialog.exportProfile:Export profile`, defaultName, filters: [{ name: $localize`:@@core.dialog.profileFilter:Debrief profile`, extensions: ['toml'] }] });
}

/** Native save dialog for an events export; null when cancelled. Browser mode returns the default name. */
export async function pickExportPath(defaultName: string, formats: { name: string; extension: string }[]): Promise<string | null> {
  if (!isTauri()) return defaultName;
  return nativePickOne({ kind: 'save', title: $localize`:@@core.dialog.exportEvents:Export events`, defaultName, filters: formats.map((f) => ({ name: f.name, extensions: [f.extension] })) });
}

/** A file type offered in a save or open dialog. */
export interface FileFilter {
  name: string;
  extensions: string[];
}

/** Native save dialog for any file the app writes; null when cancelled. Browser mode returns the default name. */
export async function pickSavePath(title: string, defaultName: string, filters: FileFilter[]): Promise<string | null> {
  if (!isTauri()) return defaultName;
  return nativePickOne({ kind: 'save', title, defaultName, filters });
}

/** Native open dialog for one file; null when cancelled. Browser mode returns `/demo/<first extension>` file. */
export async function pickOpenPath(title: string, filters: FileFilter[]): Promise<string | null> {
  if (!isTauri()) return `/demo/shared.${filters[0]?.extensions[0] ?? 'txt'}`;
  return nativePickOne({ kind: 'openFile', title, filters });
}

/**
 * Native open dialog for several files; empty when cancelled. Browser mode returns `?mockOpen=` (comma
 * separated paths, UI tests) or one `/demo/shared.<first extension>` file.
 */
export async function pickOpenPaths(title: string, filters: FileFilter[]): Promise<string[]> {
  if (!isTauri()) {
    const forced = typeof location === 'undefined' ? null : new URLSearchParams(location.search).get('mockOpen');
    return forced ? forced.split(',') : [`/demo/shared.${filters[0]?.extensions[0] ?? 'txt'}`];
  }
  return nativePick({ kind: 'openFiles', title, filters });
}

/** Injectable facade over the host functions, so components and the store can be tested with fakes. */
/**
 * Whether a path from the native save dialog was already confirmed there when it names an existing file.
 * The engine shows the dialogs with tauri-plugin-dialog, which uses rfd, whose save dialogs ask before replacing on macOS (NSSavePanel), Windows
 * (IFileSaveDialog with FOS_OVERWRITEPROMPT) and Linux (GTK overwrite confirmation / the portal). The
 * browser build has no dialog, so the app asks itself.
 */
export function saveDialogConfirmsOverwrite(): boolean {
  return isTauri();
}

@Injectable({ providedIn: 'root' })
export class Platform {
  readonly isTauri = isTauri;
  readonly saveDialogConfirmsOverwrite = saveDialogConfirmsOverwrite;
  pickPaths(directory: boolean): Promise<string[]> {
    return pickPaths(directory);
  }
  onFilesDropped(handler: (paths: string[], at: DropPoint | null) => void, hover: (over: boolean) => void): Promise<() => void> {
    return onFilesDropped(handler, hover);
  }
  pickProfileToOpen(): Promise<string | null> {
    return pickProfileToOpen();
  }
  pickProfileToSave(defaultName: string): Promise<string | null> {
    return pickProfileToSave(defaultName);
  }
  pickExportPath(defaultName: string, formats: { name: string; extension: string }[]): Promise<string | null> {
    return pickExportPath(defaultName, formats);
  }
  pickSavePath(title: string, defaultName: string, filters: FileFilter[]): Promise<string | null> {
    return pickSavePath(title, defaultName, filters);
  }
  pickOpenPath(title: string, filters: FileFilter[]): Promise<string | null> {
    return pickOpenPath(title, filters);
  }
  pickOpenPaths(title: string, filters: FileFilter[]): Promise<string[]> {
    return pickOpenPaths(title, filters);
  }
}
