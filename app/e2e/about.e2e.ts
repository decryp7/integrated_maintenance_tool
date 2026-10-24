import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test.describe('About Debrief', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('opens from the palette, shows version and licence, searches the notices', async ({ page }, testInfo) => {
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await input.fill('About Debrief');
    await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText('About Debrief');
    await page.keyboard.press('Enter');

    const dialog = page.getByRole('dialog', { name: 'About Debrief' });
    await expect(dialog).toBeVisible();
    await expect(dialog.locator('.version')).toHaveText(/^\d+\.\d+\.\d+/);
    await expect(dialog).toContainText('MIT OR Apache-2.0');

    // The notices load from a lazy chunk (no fetch, so the production CSP is fine) and are virtualised.
    const lines = dialog.locator('.line');
    await expect(lines.first()).toHaveText('# Third-party notices');
    expect(await lines.count()).toBeLessThan(80);

    const search = dialog.getByRole('searchbox', { name: 'Search notices' });
    await search.fill('serde_json');
    await expect(lines.first()).toContainText('serde_json');
    await expect(dialog.locator('.count')).toContainText(/^\d+ of \d+ lines$/);
    for (const text of await lines.allTextContents()) expect(text.toLowerCase()).toContain('serde_json');

    await expectNoSeriousViolations(page, testInfo, 'about');

    await dialog.getByRole('button', { name: 'View LICENSE-APACHE' }).click();
    await expect(dialog.getByRole('region', { name: 'LICENSE-APACHE text' })).toBeVisible();
    await search.fill('');
    await expect(lines.filter({ hasText: 'Apache License' }).first()).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
  });

  test('Help menu opens it too', async ({ page }) => {
    await chooseMenu(page, 'Help', 'About Debrief');
    await expect(page.getByRole('dialog', { name: 'About Debrief' })).toBeVisible();
  });
});

// The About window is a small bootstrap (ADR 0050): it must not drag the dock, the timeline, ECharts or any panel's code
// in. This counts every script the window loads in the browser build (production, minified), with the mock backends
// included. The third-party notices (about 780 kB of text) are no longer JavaScript: the desktop app gets them from the
// engine (`legal_text`) when the window asks, and this build shows a short stand-in. It was 1600 kB with the notices
// bundled as a lazy chunk (2424 kB before the activity observers stopped importing the dock); it is 806 kB now.
// `npm run check:dialog-weight` bounds the part the desktop app loads (static imports only, 442 kB now). Chromium runs it
// (every project tests the production build).
const ABOUT_WINDOW_JS_BUDGET_KB = 900;
test('the About window loads only a small slice of the app (JS budget)', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'chromium', 'measured once, in Chromium');
  const scripts = new Map<string, number>();
  page.on('response', async (r) => {
    if (!r.url().endsWith('.js')) return;
    scripts.set(r.url().replace(/^.*\//, ''), (await r.body().catch(() => Buffer.alloc(0))).length);
  });
  await page.goto('/#/dialog?id=about&host=main&label=d-main-1');
  await expect(page.locator('debrief-about .line').first()).toBeVisible();
  const all = [...scripts.entries()].sort((a, b) => b[1] - a[1]);
  const kb = (n: number) => Math.round(n / 1024);
  const total = all.reduce((n, [, b]) => n + b, 0);
  testInfo.annotations.push({ type: 'About window JS', description: `${kb(total)} kB in ${all.length} files` });
  const biggest = all.slice(0, 5).map(([f, b]) => `${f} ${kb(b)} kB`).join(', ');
  expect(kb(total), `About window JS: ${biggest}`).toBeLessThanOrEqual(ABOUT_WINDOW_JS_BUDGET_KB);
});
