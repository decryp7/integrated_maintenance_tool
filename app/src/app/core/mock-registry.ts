import type * as Mocks from '../mock-backends';

/** The browser build's mock backends (see `mock-backends.ts`). */
export type BrowserMocks = typeof Mocks;

let loaded: BrowserMocks | null = null;

/** Loads the mock backends. Called once before bootstrapping when not running inside Tauri. */
export async function loadBrowserMocks(): Promise<void> {
  loaded ??= await import('../mock-backends');
}

/** Installs the mock backends directly (unit tests import them statically). */
export function installBrowserMocks(mocks: BrowserMocks): void {
  loaded = mocks;
}

/** The mock backends of the browser build; fails fast when they were not loaded before the app started. */
export function browserMocks(): BrowserMocks {
  if (!loaded) throw new Error('The mock backends are not loaded: the browser build must call loadBrowserMocks() before bootstrapping.');
  return loaded;
}
