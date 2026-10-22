import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

test.describe('import progress', () => {
  test('over a workspace the status bar is the one progress surface; it opens the job in Jobs; cancelling leaves a banner', async ({ page }, testInfo) => {
    // Each opened path stands for six files read 1.5 s apart: long enough that the checks below never outlast the
    // import (Cancel would then have nothing left to cancel); cancelling early keeps the test quick.
    await openAppWith(page, '?mockSlowImport=1500&mockJobsVisibleMs=0');
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();

    const status = page.getByTestId('import-status');
    await expect(status).toBeVisible();
    await expect(status.getByRole('progressbar', { name: 'Import progress' })).toBeVisible();
    await expect(status).toContainText(/Importing \d of 6 files/);
    await expect(status).not.toContainText('0 of 0');
    await expectNoSeriousViolations(page, testInfo, 'import-status', '[data-testid=import-status]');
    // No second or third report of the same import: no card over the grid, no progress in the title bar.
    await page.waitForTimeout(1_700);
    await expect(page.getByTestId('import-toast')).toHaveCount(0);
    await expect(page.locator('imt-title-bar')).not.toContainText(/Opening|Importing/);
    await status.getByTestId('import-status-open').click();
    await expect(page.locator('imt-jobs-panel').first()).toBeVisible();
    await status.getByRole('button', { name: 'Cancel' }).click();

    const banner = page.getByTestId('partial-import-banner');
    await expect(banner).toBeVisible({ timeout: 5_000 });
    await expect(banner).toContainText(/Partially imported: [1-5] of 6 files were read/);
    await expect(status).toHaveCount(0);
    // The files not read are listed as not opened, with the reason.
    await expect(page.locator('imt-sidebar .src.dim[title="Not imported: the import was cancelled"]').first()).toBeVisible();

    await banner.getByRole('button', { name: 'Re-import all' }).click();
    await expect(banner).toHaveCount(0);
  });

  test('on the welcome page a long import also shows a card with Cancel', async ({ page }, testInfo) => {
    await openAppWith(page, '?mockSlowImport=1500&mockJobsVisibleMs=0');
    await page.keyboard.press('Control+K');
    await page.keyboard.type('Close workspace');
    await page.keyboard.press('Enter');
    await expect(page.locator('imt-title-bar')).toContainText('No folder open');
    await page.locator('imt-welcome').getByRole('button', { name: /Open files/ }).click();
    const toast = page.getByTestId('import-toast');
    await expect(toast).toBeVisible({ timeout: 5_000 });
    await expect(toast).toContainText(/Reading part-\d\.log/);
    await expect(toast).toContainText(/MB of 11 MB/);
    await expectNoSeriousViolations(page, testInfo, 'import-toast', '[data-testid=import-toast]');
    await toast.getByRole('button', { name: 'Cancel import' }).click();
    await expect(toast).toHaveCount(0, { timeout: 5_000 });
  });

  test('the status bar Cancel works too, and Dismiss hides the banner', async ({ page }) => {
    // 1.5 s per file and Cancel while at most half is read: on a slow runner the import must still be running
    // when the click lands, or there is nothing left to cancel.
    await openAppWith(page, '?mockSlowImport=1500&mockJobsVisibleMs=0');
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open folder' }).click();
    const status = page.getByTestId('import-status');
    await expect(status).toContainText(/Importing [1-3] of 6 files/, { timeout: 5_000 });
    // In a narrow window the status bar's other items give way: Cancel stays clear of the Live button (it once
    // spilled over it, and the click went to Live).
    await page.setViewportSize({ width: 1000, height: 800 });
    const cancel = await status.getByRole('button', { name: 'Cancel' }).boundingBox();
    const live = await page.getByTestId('live-toggle').boundingBox();
    expect(cancel && live && cancel.x + cancel.width <= live.x).toBe(true);
    // ...and the event count stays readable: it once shrank to nothing while the bar made room.
    const events = await page.locator('imt-status-bar span', { hasText: /\d[\d,]* events/ }).boundingBox();
    expect(events?.width ?? 0).toBeGreaterThan(30);
    await status.getByRole('button', { name: 'Cancel' }).click({ timeout: 2_000 });
    const banner = page.getByTestId('partial-import-banner');
    await expect(banner).toBeVisible({ timeout: 5_000 });
    await banner.getByRole('button', { name: 'Dismiss' }).click();
    await expect(banner).toHaveCount(0);
  });
});
