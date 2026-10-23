import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { runCommand } from './analysis-support';

test.describe('time-range import and watched folders', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('Open a time range asks for the range and imports the picked folder', async ({ page }) => {
    await page.getByTestId('open-range').click();
    const dialog = page.getByRole('dialog', { name: 'Open a time range' });
    await expect(dialog).toBeVisible();
    const open = dialog.getByTestId('range-open');
    await expect(open).toBeDisabled();
    await dialog.getByTestId('range-from').fill('2026-10-14 10:00');
    await dialog.getByTestId('range-to').fill('2026-10-14T09:00');
    await expect(dialog.getByRole('alert')).toContainText('The end is before the start.');
    await dialog.getByTestId('range-to').fill('2026-10-14T11:00');
    await expect(open).toBeEnabled();
    await open.click();
    await expect(dialog).toHaveCount(0);
    await expect(page.locator('debrief-sidebar')).toContainText('extra-folder');
  });

  test('Watched folders: a folder is added, its new file is imported, and it can be removed', async ({ page }) => {
    await runCommand(page, 'Show watched folders');
    const view = page.locator('debrief-watched-folders-view');
    await expect(view).toContainText('No folders are watched.');
    await view.getByTestId('watch-add').click();
    const item = view.getByRole('list', { name: 'Watched folders' }).getByRole('listitem');
    await expect(item).toHaveCount(1);
    await expect(item).toContainText('1 file imported');
    await expect(page.locator('debrief-sidebar')).toContainText('dropped.log');
    await view.getByRole('button', { name: /^Stop watching the folder/ }).click();
    await expect(view).toContainText('No folders are watched.');
  });
});
