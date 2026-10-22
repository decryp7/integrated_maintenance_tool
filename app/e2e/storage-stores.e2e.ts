import { expect, Page, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT, openApp } from './app';

// Settings > Storage > Project stores against the browser build's mock storage backend
// (src/app/settings/storage/mock-storage-backend.ts): five folders, two of them orphaned.

const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });

async function openStorage(page: Page) {
  await page.keyboard.press('ControlOrMeta+Comma');
  await expect(settingsDialog(page)).toBeVisible();
  await settingsDialog(page).getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Disk usage' }).click();
  await expect(settingsDialog(page).getByRole('table', { name: 'Project stores' })).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
}

test('cleans up a selected orphaned store after a confirmation and the row is gone', async ({ page }) => {
  await openApp(page);
  await openStorage(page);
  const dialog = settingsDialog(page);
  const rows = dialog.getByRole('table', { name: 'Project stores' }).locator('tbody tr');
  await expect(rows).toHaveCount(5);
  await expect(dialog.getByTestId('stores-orphaned-total')).toContainText('orphaned: 1.9 GB can be freed');

  // Stores that are open, kept for a session or used elsewhere cannot be ticked.
  await expect(dialog.getByRole('checkbox', { name: 'Select store-7f3a91c2' })).toBeDisabled();
  await expect(dialog.getByRole('button', { name: 'Clean up selected…' })).toBeDisabled();

  await dialog.getByRole('checkbox', { name: 'Select store-5e60b3f8' }).check();
  await dialog.getByRole('button', { name: 'Clean up selected…' }).click();
  const confirm = page.getByRole('dialog', { name: 'Clean up project stores?' });
  await expect(confirm.getByTestId('confirm-message')).toContainText('1 orphaned store (96 MB)');
  await expect(confirm.getByTestId('confirm-message')).toContainText('cannot be recovered');
  await confirm.getByRole('button', { name: 'Clean up' }).click();
  await expect(confirm).toHaveCount(0);

  await expect(dialog.getByTestId('stores-result')).toContainText('Freed 96 MB.');
  await expect(rows).toHaveCount(4);
  await expect(dialog.locator('tr[data-store="store-5e60b3f8"]')).toHaveCount(0);
  await expect(dialog.getByTestId('stores-orphaned-total')).toContainText('orphaned: 1.8 GB can be freed');
});
