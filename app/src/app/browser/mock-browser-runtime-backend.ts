import { BrowserRuntimeBackend } from './browser-runtime-backend';
import { RuntimeStatus } from './browser.types';

const PINNED = '131.0.6778.204';
const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * The Chromium runtime for the browser build (dev and UI tests). `?mockChromium=none` starts without any browser,
 * `installed` with a downloaded one, `outdated` with a downloaded older one; the default finds one on the machine.
 * `?mockChromiumFail=1` makes the download fail. The engine (`app/src-tauri/src/browser/runtime.rs`) is the reference.
 */
export class MockBrowserRuntimeBackend extends BrowserRuntimeBackend {
  private status: RuntimeStatus;
  private readonly fail: boolean;

  constructor(search = '') {
    super();
    const q = new URLSearchParams(search);
    this.fail = q.get('mockChromiumFail') === '1';
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

  private installed(version: string): RuntimeStatus {
    return { available: true, source: 'installed', path: '/Users/me/.local/share/debrief/chromium/chrome', installedVersion: version, pinnedVersion: PINNED, canInstall: true };
  }
}
