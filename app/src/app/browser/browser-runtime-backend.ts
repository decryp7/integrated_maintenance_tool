import { inject, Injectable } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { jobValue } from '../jobs/job-value';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { browserMocks } from '../core/mock-registry';
import { isTauri } from '../core/platform';
import { listenHere, PayloadListenFn, Unlisten } from '../windowing/window-events';
import { ResearchProfileStatus, RuntimeStatus } from './browser.types';

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
  /** The kept research profile: on or off, its size, whether a browser uses it, why it was not used. */
  abstract getResearchProfile(): Promise<ResearchProfileStatus>;
  /** Clears the kept research profile; the engine refuses while a research runs or a browser uses it. */
  abstract clearResearchProfile(): Promise<ResearchProfileStatus>;
  /** The kept research profile changed (a browser took or released it, it was cleared, the setting changed). */
  abstract onResearchProfileChanged(handler: () => void): Promise<Unlisten>;
}

/** `debrief://research-profile` (app/src-tauri/src/browser/mod.rs), to every window, no payload. */
export const RESEARCH_PROFILE_EVENT = 'debrief://research-profile';

/** Talks to `app/src-tauri/src/browser/runtime.rs`. */
export class TauriBrowserRuntimeBackend extends BrowserRuntimeBackend {
  /** `call` is Tauri's `invoke` and `listen` the window's event listener; tests pass fakes. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {
    super();
  }
  getRuntimeStatus() {
    return this.call<RuntimeStatus>('browser_runtime_status');
  }
  /** The command starts a download job and returns its id; the new status is the job's value. */
  async installRuntime() {
    const job = await this.call<number>('browser_runtime_install');
    return jobValue<RuntimeStatus>(this.call, job);
  }
  removeRuntime() {
    return this.call<RuntimeStatus>('browser_runtime_remove');
  }
  getResearchProfile() {
    return this.call<ResearchProfileStatus>('browser_research_profile_status');
  }
  clearResearchProfile() {
    return this.call<ResearchProfileStatus>('browser_research_profile_clear');
  }
  onResearchProfileChanged(handler: () => void) {
    return this.listen(RESEARCH_PROFILE_EVENT, () => handler());
  }
}

export function createBrowserRuntimeBackend(): BrowserRuntimeBackend {
  const backend: BrowserRuntimeBackend = isTauri() ? new TauriBrowserRuntimeBackend() : new (browserMocks().MockBrowserRuntimeBackend)(globalThis.location?.search ?? '');
  return withFailureLogging<BrowserRuntimeBackend>(backend, inject(Logger), { onFailure: reportBackendFailures() });
}
