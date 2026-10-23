import { expect, test } from '@playwright/test';
import { chooseMenu } from './menu-support';
import { openAppWith } from './tools-support';

test.describe('paste as log', () => {
  test('Edit > Paste as log... adds the typed text as a source in the Explorer', async ({ page }) => {
    await openAppWith(page, '');
    await chooseMenu(page, 'Edit', 'Paste as log…');
    const dialog = page.getByRole('dialog', { name: 'Paste as log' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId('paste-name')).toHaveValue('Pasted log');
    await expect(dialog.getByRole('button', { name: 'Add', exact: true })).toBeDisabled();

    await dialog.getByTestId('paste-name').fill('web01 errors');
    await dialog.getByTestId('paste-text').fill('2026-10-14 10:00:00 ERROR upstream timed out\n2026-10-14 10:00:01 INFO retrying');
    await expect(dialog.getByTestId('paste-count')).toContainText('of 8 MB used');
    await dialog.getByRole('button', { name: 'Add', exact: true }).click();
    await expect(dialog).toHaveCount(0);

    await expect(page.locator('debrief-sidebar .src', { hasText: 'web01 errors.log' })).toBeVisible();
  });
});
