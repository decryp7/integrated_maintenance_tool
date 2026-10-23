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
// in. This counts every script the window loads in the browser build (production, minified), with the mock backends and
// the notices text (749 kB, shown on open) included. It was 2424 kB when activity observers and shared UI state imported
// DockService and a mock imported the chart component (ECharts, 663 kB); it is 1600 kB now. `npm run check:dialog-weight`
// bounds the part the desktop app loads (static imports only, 437 kB now). The prod-csp project runs it.
const ABOUT_WINDOW_JS_BUDGET_KB = 1750;
test('the About window loads only a small slice of the app (JS budget)', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'prod-csp', 'the budget is for the minified production build');
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
