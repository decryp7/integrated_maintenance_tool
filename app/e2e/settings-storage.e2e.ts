import { expect, Page, test } from '@playwright/test';
import { openApp, APP_LOAD_TIMEOUT } from './app';
import { expectNoSeriousViolations } from './axe';

// Settings ▸ Storage against the browser build's mock storage backend
// (src/app/settings/storage/mock-storage-backend.ts), which mimics the engine's categories and protections.

const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const table = (page: Page) => settingsDialog(page).getByRole('table', { name: "IMT's storage" });
const row = (page: Page, id: string) => settingsDialog(page).locator(`tr[data-category="${id}"]`);

async function openStorage(page: Page) {
  await page.keyboard.press('ControlOrMeta+Comma');
  await expect(settingsDialog(page)).toBeVisible();
  await settingsDialog(page).getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Disk usage' }).click();
  await expect(table(page)).toBeVisible();
}

test('lists what IMT stores, clears the cache after a confirmation and reports what stayed', async ({ page }, testInfo) => {
  await openApp(page);
  await openStorage(page);
  await expect(table(page).locator('tbody tr')).toHaveCount(8);
  await expect(row(page, 'webview-cache')).toContainText('48 MB');
  await expect(row(page, 'logs')).toContainText("Today's log is kept.");
  await expectNoSeriousViolations(page, testInfo, 'settings-storage', '[role=dialog]');

  await settingsDialog(page).getByRole('button', { name: 'Clear cache…' }).click();
  const confirm = page.getByRole('dialog', { name: 'Clear cache?' });
  await expect(confirm.getByTestId('confirm-message')).toContainText('267 MB');
  await expect(confirm.getByTestId('confirm-detail')).toContainText('Outdated AI runtimes — 210 MB');
  await confirm.getByRole('button', { name: 'Clear cache' }).click();
  await expect(confirm).toHaveCount(0);

  const result = settingsDialog(page).getByTestId('storage-result');
  await expect(result).toContainText('Freed 267 MB.');
  await expect(result).toContainText('Web view cache: freed 45 MB, kept 1 (in use by a running program).');
  await expect(row(page, 'webview-cache')).toContainText('3 MB');
  await expect(row(page, 'ai-runtimes-outdated')).toContainText('0 bytes');
  await expect(settingsDialog(page).getByRole('button', { name: 'Clear cache…' })).toBeDisabled();
  // Data is untouched by "Clear cache".
  await expect(row(page, 'ai-models')).toContainText('4.3 GB');
});

test('a data category asks on its own; cancelling keeps it, confirming clears it', async ({ page }) => {
  await openApp(page);
  await openStorage(page);
  const clear = row(page, 'error-snapshots').getByRole('button', { name: 'Clear… (Error snapshots)' });
  await clear.click();
  const confirm = page.getByRole('dialog', { name: 'Remove Error snapshots?' });
  await expect(confirm.getByTestId('confirm-message')).toContainText('permanently deletes 3 MB');
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await expect(confirm).toHaveCount(0);
  await expect(row(page, 'error-snapshots')).toContainText('3 MB');

  await clear.click();
  await page.getByRole('dialog', { name: 'Remove Error snapshots?' }).getByRole('button', { name: 'Remove' }).click();
  await expect(settingsDialog(page).getByTestId('storage-result')).toContainText('Error snapshots: freed 3 MB.');
  await expect(row(page, 'error-snapshots')).toContainText('0 bytes');
  await expect(clear).toBeDisabled();
});

test('a failed measurement shows an inline error with Retry', async ({ page }) => {
  await page.goto('/?mockStorageFail=1');
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await page.keyboard.press('ControlOrMeta+Comma');
  await settingsDialog(page).getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Disk usage' }).click();
  const alert = settingsDialog(page).getByRole('alert');
  await expect(alert).toContainText("Could not measure IMT's storage");
  await alert.getByRole('button', { name: 'Retry' }).click();
  await expect(table(page).locator('tbody tr')).toHaveCount(8);
  await row(page, 'logs').getByRole('button', { name: 'Show in folder (IMT logs)' }).click();
  await expect(settingsDialog(page).getByRole('status').filter({ hasText: 'Opened' })).toContainText('logs');
});

test('"Clear cache…" in the command palette opens the Storage section', async ({ page }) => {
  await openApp(page);
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill('clear cache');
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText('Clear cache…');
  await page.keyboard.press('Enter');
  await expect(table(page)).toBeVisible();
  await expect(settingsDialog(page).getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Disk usage' })).toHaveAttribute('aria-current', 'page');
});
