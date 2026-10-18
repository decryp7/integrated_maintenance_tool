import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

test.describe('import progress', () => {
  test('a long import shows progress with Cancel; cancelling leaves a partially imported banner', async ({ page }, testInfo) => {
    // Each opened path stands for six files read 1.5 s apart: long enough that the axe scan of the toast never
    // outlasts the import (Cancel would then have nothing left to cancel); cancelling early keeps the test quick.
    await openAppWith(page, '?mockSlowImport=1500');
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();

    const status = page.getByTestId('import-status');
    await expect(status).toBeVisible();
    await expect(status.getByRole('progressbar', { name: 'Import progress' })).toBeVisible();
    await expect(status).toContainText(/Importing \d of 6 files/);

    const toast = page.getByTestId('import-toast');
    await expect(toast).toBeVisible({ timeout: 5_000 });
    await expect(toast).toContainText(/Reading part-\d\.log/);
    await expect(toast).toContainText(/MB of 12 MB/);
    await expectNoSeriousViolations(page, testInfo, 'import-toast', '[data-testid=import-toast]');
    await toast.getByRole('button', { name: 'Cancel import' }).click();

    const banner = page.getByTestId('partial-import-banner');
    await expect(banner).toBeVisible({ timeout: 5_000 });
    await expect(banner).toContainText(/Partially imported: [1-5] of 6 files were read/);
    await expect(status).toHaveCount(0);
    await expect(toast).toHaveCount(0);
    // The files not read are listed as not opened, with the reason.
    await expect(page.locator('imt-sidebar .src.dim[title="Not imported: the import was cancelled"]').first()).toBeVisible();

    await banner.getByRole('button', { name: 'Re-import all' }).click();
    await expect(banner).toHaveCount(0);
  });

  test('the status bar Cancel works too, and Dismiss hides the banner', async ({ page }) => {
    // 1.5 s per file and Cancel while at most half is read: on a slow runner the import must still be running
    // when the click lands, or there is nothing left to cancel.
    await openAppWith(page, '?mockSlowImport=1500');
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open folder' }).click();
    const status = page.getByTestId('import-status');
    await expect(status).toContainText(/Importing [1-3] of 6 files/, { timeout: 5_000 });
    await status.getByRole('button', { name: 'Cancel' }).click();
    const banner = page.getByTestId('partial-import-banner');
    await expect(banner).toBeVisible({ timeout: 5_000 });
    await banner.getByRole('button', { name: 'Dismiss' }).click();
    await expect(banner).toHaveCount(0);
  });
});
