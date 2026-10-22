import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';
import { RuntimeStatus } from './browser.types';

/**
 * Which Chromium research and the browser modes drive (Settings ▸ Web browser ▸ Browser for research). The engine
 * decides (a configured path, the downloaded Chromium, or one found on the machine); the UI shows it. Injected as a
 * class token: the Tauri commands in the app, the in-memory mock in a plain browser.
 */
@Injectable({ providedIn: 'root', useFactory: createBrowserRuntimeBackend })
export abstract class BrowserRuntimeBackend {
  abstract getRuntimeStatus(): Promise<RuntimeStatus>;
  /** Downloads the pinned Chromium (only on the user's request; can take minutes); resolves with the new status. */
  abstract installRuntime(): Promise<RuntimeStatus>;
  /** Removes the downloaded Chromium (the machine's own browsers are never touched); resolves with the new status. */
  abstract removeRuntime(): Promise<RuntimeStatus>;
}

/** Talks to `app/src-tauri/src/browser/runtime.rs`. */
export class TauriBrowserRuntimeBackend extends BrowserRuntimeBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }
  getRuntimeStatus() {
    return this.call<RuntimeStatus>('browser_runtime_status');
  }
  installRuntime() {
    return this.call<RuntimeStatus>('browser_runtime_install');
  }
  removeRuntime() {
    return this.call<RuntimeStatus>('browser_runtime_remove');
  }
}

export function createBrowserRuntimeBackend(): BrowserRuntimeBackend {
  const backend: BrowserRuntimeBackend = isTauri() ? new TauriBrowserRuntimeBackend() : new (browserMocks().MockBrowserRuntimeBackend)(globalThis.location?.search ?? '');
  return withFailureLogging<BrowserRuntimeBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
