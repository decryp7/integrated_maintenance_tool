import { existsSync } from 'node:fs';
import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations, settleAnimations } from './axe';

/**
 * Every built-in theme in both colour schemes (ADR 0018): the main window passes axe (WCAG 2.2 AA), and the main
 * window and the timeline match their screenshots. Screenshots are taken in Chromium only, on the mock backend's
 * deterministic demo data, at a fixed viewport, with animations off and reduced motion on.
 *
 * Baselines are per platform (fonts render differently on Windows, Linux and macOS). Where none exists for this
 * platform yet the screenshot test is skipped; create them with `npx playwright test themes --project=chromium
 * --update-snapshots` and commit the PNGs from `e2e/themes.e2e.ts-snapshots/`.
 */
const THEMES = ['calm', 'console'] as const;
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
        // The load time in the title bar differs from run to run.
        const mask = [page.locator('imt-title-bar .state')];
        await expect(page).toHaveScreenshot(shots[0]!, { animations: 'disabled', caret: 'hide', mask, maxDiffPixelRatio: 0.01 });
        // The close-up: the timeline maximised, every lane in view.
        const group = page.locator('.dv-groupview').filter({ has: page.getByRole('tab', { name: 'Timeline', exact: true }) });
        await group.getByRole('button', { name: 'Maximize panel' }).click();
        await expect(page.locator('imt-timeline-panel .lane')).toHaveCount(7);
        await settleAnimations(page);
        await expect(page.locator('imt-timeline-panel')).toHaveScreenshot(shots[1]!, { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.01 });
      });
    });
  }
}

test('switching the theme restyles the timeline: runs, lane fill and surface follow the traits', async ({ page }) => {
  await openThemed(page, 'calm', 'dark');
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-runs', 'line');
  await expect(html).toHaveAttribute('data-lane-fill', 'neutral');
  const run = page.locator('imt-timeline-panel imt-run-track .run[data-status="closed"]').first();
  await expect(run).toBeVisible();
  // A run is a line in Calm (no fill) and a pill in Console (a tinted, rounded band).
  expect(await run.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('theme: console');
  await page.keyboard.press('Enter');
  await expect(html).toHaveAttribute('data-runs', 'pill');
  await expect(html).toHaveAttribute('data-surface', 'cards');
  await expect.poll(() => run.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
  expect(await run.evaluate((el) => getComputedStyle(el).borderRadius)).toBe('999px');
  // The lane's volume is drawn in its source colour.
  await expect(html).toHaveAttribute('data-lane-fill', 'bySource');
  const edge = page.locator('imt-timeline-panel .lane.colored .density .edge').first();
  expect(await edge.evaluate((el) => getComputedStyle(el).stroke)).not.toBe('none');
});
