import { existsSync } from 'node:fs';
import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations, settleAnimations, settleDockLayout } from './axe';

/**
 * Every built-in theme in both colour schemes (ADR 0018): the main window passes axe (WCAG 2.2 AA), and the main
 * window and the timeline match their screenshots. Screenshots are taken in Chromium only, on the mock backend's
 * deterministic demo data, at a fixed viewport, with animations off and reduced motion on.
 *
 * Baselines are per platform (fonts render differently on Windows, Linux and macOS). Where none exists for this
 * platform yet the screenshot test is skipped; create them with `npx playwright test themes --project=chromium
 * --update-snapshots` and commit the PNGs from `e2e/themes.e2e.ts-snapshots/`.
 */
const THEMES = ['calm', 'console', 'precision', 'contrast'] as const;
const SCHEMES = ['light', 'dark'] as const;

async function openThemed(page: Page, theme: string, scheme: 'light' | 'dark'): Promise<void> {
  await page.addInitScript(([t, s]) => localStorage.setItem('imt.prefs.v1', JSON.stringify({ theme: t, scheme: s })), [theme, scheme]);
  await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
  await openApp(page);
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-theme', theme);
  await expect(html).toHaveAttribute('data-scheme', scheme);
  await page.evaluate(() => document.fonts.ready);
}

for (const theme of THEMES) {
  for (const scheme of SCHEMES) {
    test.describe(`${theme} ${scheme}`, () => {
      test('the main window has no serious accessibility violations', async ({ page }, testInfo) => {
        await openThemed(page, theme, scheme);
        await page.locator('imt-events-grid [role=row].row').first().click();
        await expectNoSeriousViolations(page, testInfo, `theme-${theme}-${scheme}`);
      });

      test('the main window and the timeline match their screenshots', async ({ page }, testInfo) => {
        test.skip(testInfo.project.name !== 'chromium', 'screenshots are compared in Chromium only');
        const shots = [`main-${theme}-${scheme}.png`, `timeline-${theme}-${scheme}.png`];
        const updating = testInfo.config.updateSnapshots === 'all' || testInfo.config.updateSnapshots === 'changed';
        test.skip(!updating && !shots.every((s) => existsSync(testInfo.snapshotPath(s))), `no screenshot baselines for ${process.platform} yet (run with --update-snapshots)`);
        await openThemed(page, theme, scheme);
        await settleAnimations(page);
        await settleDockLayout(page);
        // The load time (the events count's tooltip) is not drawn; nothing in the chrome differs from run to run.
        const mask: ReturnType<typeof page.locator>[] = [];
        await expect(page).toHaveScreenshot(shots[0]!, { animations: 'disabled', caret: 'hide', mask, maxDiffPixelRatio: 0.01 });
        // The close-up: the timeline maximised, every lane in view.
        const group = page.locator('.dv-groupview').filter({ has: page.getByRole('tab', { name: 'Timeline', exact: true }) });
        await group.getByRole('button', { name: 'Maximize panel' }).click();
        await expect(page.locator('imt-timeline-panel .lane')).toHaveCount(7);
        await settleAnimations(page);
        await settleDockLayout(page);
        await expect(page.locator('imt-timeline-panel')).toHaveScreenshot(shots[1]!, { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.01 });
      });
    });
  }
}

test('switching the theme restyles the timeline: runs, lane fill and surface follow the traits', async ({ page }) => {
  await openThemed(page, 'calm', 'dark');
  // The checks read final styles; headless WebKit does not advance transitions, so a switched value would stay at its
  // start.
  await page.addStyleTag({ content: '*, *::before, *::after { transition: none !important; }' });
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-runs', 'line');
  await expect(html).toHaveAttribute('data-lane-fill', 'neutral');
  // A run cut off at the edge of the view squares that end, so pick one shown whole.
  const run = page.locator('imt-timeline-panel imt-run-track .run[data-status="closed"]:not(.clip-l):not(.clip-r)').first();
  await expect(run).toBeVisible();
  // A run is a line in Calm (no fill) and a pill in Console (a tinted, rounded band).
  expect(await run.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('theme: console');
  await page.keyboard.press('Enter');
  await expect(html).toHaveAttribute('data-runs', 'pill');
  await expect(html).toHaveAttribute('data-surface', 'cards');
  await expect.poll(() => run.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
  // One corner (WebKit leaves the `border-radius` shorthand empty in computed styles), polled: the switch transitions it.
  await expect.poll(() => run.evaluate((el) => getComputedStyle(el).borderTopLeftRadius)).toBe('999px');
  // The lane's volume is drawn in its source colour.
  await expect(html).toHaveAttribute('data-lane-fill', 'bySource');
  const edge = page.locator('imt-timeline-panel .lane.colored .density .edge').first();
  expect(await edge.evaluate((el) => getComputedStyle(el).stroke)).not.toBe('none');
});

test('Precision draws bracket runs and barcode lanes; the interface font follows the Appearance setting', async ({ page }) => {
  await openThemed(page, 'precision', 'light');
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-runs', 'bracket');
  await expect(html).toHaveAttribute('data-lane-fill', 'barcode');
  await expect(page.locator('imt-timeline-panel .lane .density .bars').first()).toBeVisible();
  // The operating system's font by default (native feel, ADR 0028), log text in the theme's mono face.
  const font = () => page.locator('imt-status-bar').evaluate((el) => getComputedStyle(el).fontFamily);
  expect(await font()).toContain('Segoe UI');
  const time = page.locator('imt-events-grid [role=row].row .time').first();
  expect(await time.evaluate((el) => getComputedStyle(el).fontFamily)).toContain('IBM Plex Mono');
  // Keeping the theme's font (Settings ▸ Appearance ▸ Interface font) brings IBM Plex Sans to the interface.
  const themed = await page.context().newPage();
  await themed.addInitScript(() => localStorage.setItem('imt.prefs.v1', JSON.stringify({ theme: 'precision', scheme: 'light', uiFont: 'theme' })));
  await openApp(themed);
  expect(await themed.locator('imt-status-bar').evaluate((el) => getComputedStyle(el).fontFamily)).toContain('IBM Plex Sans');
});
