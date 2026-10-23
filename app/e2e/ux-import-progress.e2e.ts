import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';
import { runCommand } from './tools-support';

/**
 * Each opened path stands for six files read a simulated minute apart, on Playwright's clock: the import is still
 * running whenever the test looks (however slow the runner), and `page.clock.runFor` reads the next file when the test
 * wants one read (the mock checks for a cancel between files, so a cancel takes effect once the current file is read).
 */
const SLOW_IMPORT = '?mockSlowImport=60000&mockJobsVisibleMs=0';
const FILE_MS = 60_000;

test.describe('import progress', () => {
  test('over a workspace the status bar is the one progress surface; it opens the job in Jobs; cancelling leaves a banner', async ({ page }, testInfo) => {
    await page.clock.install();
    await openAppWith(page, SLOW_IMPORT);
    await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();

    const status = page.getByTestId('import-status');
    await expect(status).toBeVisible();
    await expect(status.getByRole('progressbar', { name: 'Import progress' })).toBeVisible();
    await expect(status).toContainText('Importing 0 of 6 files');
    await expect(status).not.toContainText('0 of 0');
    await expectNoSeriousViolations(page, testInfo, 'import-status', '[data-testid=import-status]');
    // Long past the point where a long import gets a card: still no second or third report of the same import, no
    // card over the grid and no progress in the title bar.
    await page.clock.runFor(FILE_MS);
    await expect(status).toContainText('Importing 1 of 6 files');
    await expect(page.getByTestId('import-toast')).toHaveCount(0);
    await expect(page.locator('debrief-title-bar')).not.toContainText(/Opening|Importing/);
    await status.getByTestId('import-status-open').click();
    await expect(page.locator('debrief-jobs-panel').first()).toBeVisible();
    await status.getByRole('button', { name: 'Cancel' }).click();
    await page.clock.runFor(FILE_MS);

    const banner = page.getByTestId('partial-import-banner');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('Partially imported: 2 of 6 files were read');
    await expect(status).toHaveCount(0);
    // The files not read are listed as not opened, with the reason.
    await expect(page.locator('debrief-sidebar .src.dim[title="Not imported: the import was cancelled"]').first()).toBeVisible();

    await banner.getByRole('button', { name: 'Re-import all' }).click();
    await expect(banner).toHaveCount(0);
  });

  test('on the welcome page a long import also shows a card with Cancel', async ({ page }, testInfo) => {
    await page.clock.install();
    await openAppWith(page, SLOW_IMPORT);
    // Typed only once the palette is open and lists the command: keys pressed before that are lost.
    await runCommand(page, 'Close workspace');
    await expect(page.locator('debrief-title-bar')).toContainText('No folder open');
    await page.locator('debrief-welcome').getByRole('button', { name: /Open files/ }).click();
    const toast = page.getByTestId('import-toast');
    await page.clock.runFor(FILE_MS);
    await expect(toast).toBeVisible();
    await expect(toast).toContainText('Reading part-2.log');
    await expect(toast).toContainText(/MB of 11 MB/);
    await expectNoSeriousViolations(page, testInfo, 'import-toast', '[data-testid=import-toast]');
    await toast.getByRole('button', { name: 'Cancel import' }).click();
    await page.clock.runFor(FILE_MS);
    await expect(toast).toHaveCount(0);
  });

  test('the status bar Cancel works too, and Dismiss hides the banner', async ({ page }) => {
    await page.clock.install();
    await openAppWith(page, SLOW_IMPORT);
    await page.locator('debrief-sidebar').getByRole('button', { name: 'Open folder' }).click();
    const status = page.getByTestId('import-status');
    await expect(status).toContainText('Importing 0 of 6 files');
    // In a narrow window the status bar's other items give way: Cancel stays clear of the Live button (it once
    // spilled over it, and the click went to Live).
    await page.setViewportSize({ width: 1000, height: 800 });
    const cancel = await status.getByRole('button', { name: 'Cancel' }).boundingBox();
    const live = await page.getByTestId('live-toggle').boundingBox();
    expect(cancel && live && cancel.x + cancel.width <= live.x).toBe(true);
    // ...and the event count stays readable: it once shrank to nothing while the bar made room.
    const events = await page.locator('debrief-status-bar span', { hasText: /\d[\d,]* events/ }).boundingBox();
    expect(events?.width ?? 0).toBeGreaterThan(30);
    await status.getByRole('button', { name: 'Cancel' }).click();
    await page.clock.runFor(FILE_MS);
    const banner = page.getByTestId('partial-import-banner');
    await expect(banner).toBeVisible();
    await banner.getByRole('button', { name: 'Dismiss' }).click();
    await expect(banner).toHaveCount(0);
  });

  test('the phases explain themselves: a growing found count, then files; Cancel works while finding', async ({ page }) => {
    await page.clock.install();
    await openAppWith(page, `${SLOW_IMPORT}&mockSlowDiscovery=60000`);
    await page.locator('debrief-sidebar').getByRole('button', { name: 'Open folder' }).click();
    const status = page.getByTestId('import-status');
    // From the first moment: a count (not an idle 0), growing with every step.
    await expect(status).toContainText('Finding files… 1,234 found');
    const first = await status.innerText();
    await page.clock.runFor(FILE_MS);
    await expect(status).toContainText('Finding files… 2,468 found');
    expect(await status.innerText()).not.toBe(first);
    await expect(status).toContainText('folder-2');
    await page.clock.runFor(3 * FILE_MS);
    await expect(status).toContainText('Finding files… 6,170 found');
    await page.clock.runFor(FILE_MS);
    await expect(status).toContainText('Importing 0 of 6 files');
    await status.getByRole('button', { name: 'Cancel' }).click();
    await page.clock.runFor(FILE_MS);
    await expect(page.getByTestId('partial-import-banner')).toBeVisible();
  });

  test('Cancel during the finding phase stops the import without reading files', async ({ page }) => {
    await page.clock.install();
    await openAppWith(page, `${SLOW_IMPORT}&mockSlowDiscovery=60000`);
    await page.locator('debrief-sidebar').getByRole('button', { name: 'Open folder' }).click();
    const status = page.getByTestId('import-status');
    await expect(status).toContainText('Finding files…');
    await status.getByRole('button', { name: 'Cancel' }).click();
    await page.clock.runFor(FILE_MS);
    await expect(status).toHaveCount(0);
    await expect(page.getByTestId('partial-import-banner')).toBeVisible();
  });
});
