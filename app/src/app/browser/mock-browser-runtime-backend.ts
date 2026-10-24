import { BrowserRuntimeBackend } from './browser-runtime-backend';
import { ResearchProfileStatus, RuntimeStatus } from './browser.types';
import { Unlisten } from '../windowing/window-events';

const PINNED = '131.0.6778.204';
const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * The Chromium runtime for the browser build (dev and UI tests). `?mockChromium=none` starts without any browser,
 * `installed` with a downloaded one, `outdated` with a downloaded older one; the default finds one on the machine.
 * `?mockChromiumFail=1` makes the download fail. `?mockResearchProfile=on` keeps a research profile (`busy`: in use,
 * `damaged`: it was not used). The engine (`app/src-tauri/src/browser/runtime.rs`) is the reference.
 */
export class MockBrowserRuntimeBackend extends BrowserRuntimeBackend {
  private status: RuntimeStatus;
  private readonly fail: boolean;
  private profile: ResearchProfileStatus;
  private readonly profileListeners = new Set<() => void>();

  constructor(search = '') {
    super();
    const q = new URLSearchParams(search);
    this.fail = q.get('mockChromiumFail') === '1';
    const kept = q.get('mockResearchProfile');
    this.profile = {
      keep: !!kept,
      sizeBytes: kept ? 12 << 20 : 0,
      maxBytes: 500 << 20,
      inUse: kept === 'busy',
      problem: kept === 'damaged' ? 'The research profile is damaged (Default/Preferences cannot be read), so this research used a throwaway profile. Clear it in Settings ▸ Web browser to start a fresh one.' : null,
    };
    const start = q.get('mockChromium');
    this.status =
      start === 'none'
        ? { available: false, source: 'none', path: null, installedVersion: null, pinnedVersion: PINNED, canInstall: true }
        : start === 'installed' || start === 'outdated'
          ? this.installed(start === 'outdated' ? '130.0.6723.58' : PINNED)
          : { available: true, source: 'found', path: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', installedVersion: null, pinnedVersion: PINNED, canInstall: true };
  }

  getRuntimeStatus() {
    return Promise.resolve({ ...this.status });
  }

  async installRuntime() {
    await delay(40);
    if (this.fail) throw new Error('the Chromium download failed: the server did not answer');
    this.status = this.installed(PINNED);
    return { ...this.status };
  }

  async removeRuntime() {
    await delay(0);
    this.status = { available: true, source: 'found', path: '/usr/bin/chromium', installedVersion: null, pinnedVersion: PINNED, canInstall: true };
    return { ...this.status };
  }

  getResearchProfile() {
    return Promise.resolve({ ...this.profile });
  }

  async clearResearchProfile() {
    await delay(0);
    if (this.profile.inUse) throw new Error('The research profile is in use by the AI browser. Close its window (or wait for the research to finish), then clear it.');
    this.profile = { ...this.profile, sizeBytes: 0, problem: null };
    this.profileChanged();
    return { ...this.profile };
  }

  onResearchProfileChanged(handler: () => void): Promise<Unlisten> {
    this.profileListeners.add(handler);
    return Promise.resolve(() => this.profileListeners.delete(handler));
  }

  /** Plays the engine: the kept profile changed (`patch`), and every window hears it. */
  setResearchProfile(patch: Partial<ResearchProfileStatus>): void {
    this.profile = { ...this.profile, ...patch };
    this.profileChanged();
  }

  private profileChanged(): void {
    for (const f of this.profileListeners) f();
  }

  private installed(version: string): RuntimeStatus {
    return { available: true, source: 'installed', path: '/Users/me/.local/share/debrief/chromium/chrome', installedVersion: version, pinnedVersion: PINNED, canInstall: true };
  }
}
