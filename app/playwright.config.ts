import { defineConfig, devices } from '@playwright/test';

// Ports and the production-build server can be overridden so several runs can share a machine.
const PORT = Number(process.env['IMT_E2E_PORT'] ?? 4300);
const PROD_PORT = Number(process.env['IMT_E2E_PROD_PORT'] ?? 4301);
const SKIP_PROD = !!process.env['IMT_E2E_SKIP_PROD'];
const CI = !!process.env['CI'];
const PROD_CSP = '**/prod-csp.e2e.ts';
// These also run against the production build with the Tauri CSP (no extra permissions needed).
const ERROR_SNAPSHOTS = '**/error-snapshots.e2e.ts';
const SETTINGS = '**/settings.e2e.ts';
const WINDOWING = '**/windowing.e2e.ts';
const ABOUT = '**/about.e2e.ts';
const DOCKING = ['**/popout*.e2e.ts', '**/dock*.e2e.ts'];
const MENUS = '**/menus*.e2e.ts';

// End-to-end tests run against the browser build, where the app injects the in-memory MockBackend.
// The "prod-csp" project runs against the production build served with the Tauri CSP header.
export default defineConfig({
  testDir: 'e2e',
  testMatch: '**/*.e2e.ts',
  fullyParallel: true,
  forbidOnly: CI,
  retries: CI ? 2 : 0,
  // Two on CI: each shard's container shares a runner host with other jobs; three made WebKit tests time out.
  workers: CI ? 2 : undefined,
  reporter: [['html', { open: 'never' }], ['list']],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: 'on-first-retry',
    viewport: { width: 1440, height: 900 },
  },
  projects: [
    { name: 'chromium', testIgnore: PROD_CSP, use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } },
    { name: 'webkit', testIgnore: PROD_CSP, use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 900 } } },
    ...(SKIP_PROD
      ? []
      : [
          {
            name: 'prod-csp',
            testMatch: [PROD_CSP, ERROR_SNAPSHOTS, SETTINGS, WINDOWING, ABOUT, MENUS, ...DOCKING, '**/live-*.e2e.ts', '**/share-*.e2e.ts', '**/profiling-*.e2e.ts', '**/ux-*.e2e.ts', '**/ai-*.e2e.ts', '**/i18n-*.e2e.ts'],
            use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 }, baseURL: `http://localhost:${PROD_PORT}` },
          },
        ]),
  ],
  webServer: [
    {
      // No file watching: an edit made while the suite runs (another terminal, another agent) would reload every
      // open page through HMR mid-test and fail whatever was running. Tests run against the build they started with.
      command: `npx ng serve --port ${PORT} --watch=false`,
      url: `http://localhost:${PORT}`,
      reuseExistingServer: !CI,
      timeout: 180_000,
    },
    ...(SKIP_PROD
      ? []
      : [{
      // Always rebuild so the guard tests the current production configuration.
      command: `npx ng build && node e2e/serve-prod.mjs ${PROD_PORT}`,
      url: `http://localhost:${PROD_PORT}`,
      reuseExistingServer: false,
      timeout: 300_000,
    }]),
  ],
});
