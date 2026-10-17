/** Stops listening to an event. */
export type Unlisten = () => void;

/** Subscribes to an event with its payload; tests pass fakes. */
export type PayloadListenFn = (event: string, handler: (payload: unknown) => void) => Promise<Unlisten>;

/** Label of the window this webview belongs to (Tauri only; the window API is loaded lazily). */
export async function currentWindowLabel(): Promise<string> {
  const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  return getCurrentWebviewWindow().label;
}

/**
 * Listens to a Tauri event addressed to this window. Without a target Tauri 2 delivers events of
 * every window; a string target (`AnyLabel`) matches `emit_to(label)` and still receives global emits.
 * The event API is loaded lazily so the browser build does not need it.
 */
export async function listenHere(event: string, handler: (payload: unknown) => void): Promise<Unlisten> {
  const [{ listen }, target] = await Promise.all([import('@tauri-apps/api/event'), currentWindowLabel()]);
  return listen(event, (e) => handler(e?.payload), { target });
}
