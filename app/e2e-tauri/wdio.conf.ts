import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { browser } from '@wdio/globals';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { connect } from 'node:net';
import { platform, tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  appBinary,
  ARTIFACTS_DIR,
  nativeDriver,
  RUN_DIR_VAR,
  runLayout,
  SAMPLES,
  tauriDriver,
} from './support/paths';

// Smoke tests of the real IMT app (Rust engine + Tauri shell + production UI build, no mock backend), driven by
// WebdriverIO through tauri-driver: WebView2 via msedgedriver on Windows, WebKitGTK via WebKitWebDriver on
// Linux. macOS has no WebDriver for WKWebView, so these do not run there. See docs/testing-manual.md.
//
//   npm run e2e:tauri:build     # debug app binary with the frontend built in
//   npm run e2e:tauri           # the specs (one app session per spec file, run in order)

const WINDOWS = platform() === 'win32';
const app = appBinary();
let driver: ChildProcess | undefined;

// Every run keeps IMT's data, config and logs and the webview's profile in a fresh temp folder, never the
// user's: debug builds take that root from IMT_DATA_DIR (app_dirs.rs). The launcher creates the folder; the
// workers load this file again with the launcher's environment, so they reuse it. `onComplete` removes it.
const runDir = (process.env[RUN_DIR_VAR] ??= mkdtempSync(join(tmpdir(), 'imt-e2e-')));
const layout = runLayout(runDir);
process.env['IMT_DATA_DIR'] = layout.imt;
if (WINDOWS) process.env['IMT_OPEN_PATHS'] = SAMPLES;
if (!WINDOWS) {
  // Linux: WebKitGTK and GTK also write to the XDG folders.
  process.env['XDG_DATA_HOME'] = layout.xdg.data;
  process.env['XDG_CONFIG_HOME'] = layout.xdg.config;
  process.env['XDG_CACHE_HOME'] = layout.xdg.cache;
}

/**
 * Whether this Windows process runs elevated (High or System integrity). WebView2 ignores the
 * WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS variable (and HKCU overrides) for elevated hosts, and that variable is how
 * msedgedriver turns on the remote debugging port. Debug builds of IMT hand the value to WebView2 through its
 * API themselves (dev_overrides.rs), so only a release binary cannot be driven from an elevated terminal.
 */
function elevated(): boolean {
  try {
    const groups = execFileSync('whoami', ['/groups'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    return /S-1-16-(12288|16384)\b/.test(groups);
  } catch {
    return false;
  }
}

/** Stops the run before any session starts (an error thrown from `onPrepare` is only logged). */
function fail(message: string): never {
  console.error(`\n${message}\n`);
  process.exit(1);
}

/** Resolves once something listens on the port (tauri-driver takes a moment to bind). */
async function waitForPort(port: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const open = await new Promise<boolean>((resolve) => {
      const s = connect({ host: '127.0.0.1', port }, () => (s.end(), resolve(true)));
      s.once('error', () => resolve(false));
    });
    if (open) return;
    if (Date.now() > deadline)
      throw new Error(`tauri-driver did not listen on port ${port} within ${timeoutMs} ms`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

export const config: WebdriverIO.Config = {
  runner: 'local',
  hostname: '127.0.0.1',
  port: 4444,
  specs: ['./specs/**/*.e2e.ts'],
  maxInstances: 1,
  capabilities: [
    {
      // Every spec starts with the CLEF samples open: on Linux they are on the command line; msedgedriver turns
      // every argument into a `--switch`, so on Windows the debug build takes them from IMT_OPEN_PATHS (above).
      'tauri:options': { application: app, args: WINDOWS ? [] : [SAMPLES] },
    } as WebdriverIO.Capabilities,
  ],
  logLevel: 'warn',
  bail: 0,
  waitforTimeout: 15_000,
  connectionRetryTimeout: 120_000,
  // A retry would launch a second app, which the single-instance lock hands to the first one.
  connectionRetryCount: 0,
  framework: 'mocha',
  reporters: ['spec'],
  // Each test has a two-minute budget (most finish in seconds).
  mochaOpts: { ui: 'bdd', timeout: 120_000 },

  onPrepare() {
    if (!existsSync(app))
      fail(
        `No IMT binary at ${app}. Build it first: npm run e2e:tauri:build (or set IMT_E2E_APP).`,
      );
    const td = tauriDriver();
    if (!existsSync(td))
      fail(
        `No tauri-driver at ${td}. Install it: cargo install tauri-driver --locked (or set TAURI_DRIVER).`,
      );
    const native = nativeDriver();
    if (native && !existsSync(native))
      fail(`No msedgedriver at ${native}. Fetch it: node e2e-tauri/scripts/fetch-msedgedriver.mjs`);
    if (WINDOWS && elevated())
      console.warn(
        'This terminal runs as administrator: the session relies on the debug build handing ' +
          'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS to WebView2 itself. A release binary cannot be driven elevated ' +
          '("DevToolsActivePort file doesn\'t exist"): build with npm run e2e:tauri:build or use a non-elevated terminal.',
      );
    mkdirSync(layout.imt, { recursive: true });
    if (!WINDOWS) for (const d of Object.values(layout.xdg)) mkdirSync(d, { recursive: true });
    console.log(`IMT data for this run: ${layout.imt}`);
  },

  // One tauri-driver per spec file (sessions run one after another).
  async beforeSession() {
    const native = nativeDriver();
    const args = native ? ['--native-driver', native] : [];
    driver = spawn(tauriDriver(), args, { stdio: [null, process.stdout, process.stderr] });
    await waitForPort(4444, 20_000);
  },

  // Every spec ends with a clean quit, so the next one does not start after a "crash" (a killed app leaves its
  // running marker behind). Specs that already quit are skipped.
  async after() {
    const { quitApp, isRunning } = await import('./support/app');
    if (await isRunning()) await quitApp();
  },

  // A failed test leaves a screenshot and the page's HTML in e2e-tauri/.artifacts/ (uploaded by CI).
  async afterTest(test, _context, { passed }) {
    if (passed) return;
    const name = `${test.parent} ${test.title}`.replace(/[^\w.-]+/g, '_').slice(0, 120);
    mkdirSync(ARTIFACTS_DIR, { recursive: true });
    try {
      await browser.saveScreenshot(join(ARTIFACTS_DIR, `${name}.png`));
      writeFileSync(join(ARTIFACTS_DIR, `${name}.html`), await browser.getPageSource());
    } catch (e) {
      console.warn(`No failure artifacts for "${name}": ${String(e)}`);
    }
  },

  afterSession() {
    driver?.kill();
    driver = undefined;
  },

  // The run's folder goes with it (WebView2 can hold files for a moment after the app exits); set
  // IMT_E2E_KEEP_DATA=1 to keep it (IMT's logs are in its `logs` folder).
  onComplete() {
    if (process.env['IMT_E2E_KEEP_DATA'])
      return console.log(`Kept IMT data of this run: ${layout.imt}`);
    rmSync(runDir, { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
  },
};
