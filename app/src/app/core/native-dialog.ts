import { invoke } from './activity/ipc';
import { PickRequest } from './api.types';

/**
 * The native open and save dialogs. They run in the engine (`pick_paths`), which records what the user
 * chose as a grant for this window: commands that read or write a path the webview names accept only
 * chosen ones. The dialog plugin's own JavaScript API is not used (and not permitted).
 */
/* v8 ignore start: runs only inside Tauri; the browser build never calls it */
export async function nativePick(request: PickRequest): Promise<string[]> {
  return invoke<string[]>('pick_paths', { request });
}

/** One path (the first chosen), or null when cancelled. */
export async function nativePickOne(request: PickRequest): Promise<string | null> {
  return (await nativePick(request))[0] ?? null;
}
/* v8 ignore stop */
