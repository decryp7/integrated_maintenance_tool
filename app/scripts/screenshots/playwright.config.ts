import { defineConfig, devices } from '@playwright/test';

// Regenerates the README screenshots (docs/screenshots) from the browser build with the mock backend.
// Run from app/: npm run screenshots   (needs `npm run e2e:install` once for the Chromium browser)
const PORT = Number(process.env['DEBRIEF_SHOT_PORT'] ?? 4320);

export default defineConfig({
  testDir: '.',
  testMatch: 'capture.ts',
  workers: 1,
  timeout: 300_000,
  reporter: [['list']],
  use: {
    ...devices['Desktop Chrome'],
    baseURL: `http://localhost:${PORT}`,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,
    actionTimeout: 8_000,
    navigationTimeout: 30_000,
  },
  webServer: {
    command: `npx ng serve --port ${PORT}`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: true,
    timeout: 180_000,
  },
});
