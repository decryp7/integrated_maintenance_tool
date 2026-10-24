import { defineConfig, devices } from '@playwright/test';

// End-to-end tests run against the browser build, where the app injects the in-memory MockBackend. Every project uses
// the one production build, served with the Tauri CSP header (e2e/serve-prod.mjs): one build per run (and per CI shard)
// instead of a dev server plus a production build, and every spec runs under the CSP the desktop app enforces.
const CI = !!process.env['CI'];
// CI must name its port (a fixed default could collide with another job on the host); locally 4300 is the default.
if (CI && !process.env['DEBRIEF_E2E_PORT']) throw new Error('CI: set DEBRIEF_E2E_PORT (the port of the production-build server)');
const PORT = Number(process.env['DEBRIEF_E2E_PORT'] ?? 4300);
// DEBRIEF_E2E_PREBUILT=1 serves the dist/app/browser an earlier `npx ng build` left (CI builds once before the tests);
// otherwise the run builds first, so it never tests a stale build.
const PREBUILT = !!process.env['DEBRIEF_E2E_PREBUILT'];
// The CSP guard spec runs in Chromium only.
const PROD_CSP = '**/prod-csp.e2e.ts';

export default defineConfig({
  testDir: 'e2e',
  testMatch: '**/*.e2e.ts',
  fullyParallel: true,
  forbidOnly: CI,
  retries: CI ? 2 : 0,
  // Multi-step flows (tour, live tail) outlast 30 s of wall clock on a shared WebKit runner without being broken.
  timeout: CI ? 60_000 : 30_000,
  // Two on CI: each shard's container shares a runner host with other jobs; three made WebKit tests time out.
  workers: CI ? 2 : undefined,
  reporter: [['html', { open: 'never' }], ['list'], ['./e2e/retried-reporter.mjs']],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: 'on-first-retry',
    viewport: { width: 1440, height: 900 },
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } },
    { name: 'webkit', testIgnore: PROD_CSP, use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 900 } } },
  ],
  webServer: {
    command: PREBUILT ? `node e2e/serve-prod.mjs ${PORT}` : `npx ng build && node e2e/serve-prod.mjs ${PORT}`,
    url: `http://localhost:${PORT}`,
    // Never reuse whatever listens on the port: a stale server would test the wrong build.
    reuseExistingServer: false,
    timeout: 300_000,
  },
});
