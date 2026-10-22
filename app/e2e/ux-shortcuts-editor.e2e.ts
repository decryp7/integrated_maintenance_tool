import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

// The shortcut editor (Settings ▸ Keyboard) and the cheat sheet that follows it. Overrides live in localStorage.

const ALT_J = /Alt\+J|⌥J/;

async function openEditor(page: Page) {
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('Edit keyboard shortcuts');
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog.getByRole('searchbox', { name: 'Search shortcuts' })).toBeVisible();
  return dialog;
}

test.describe('shortcut editor', () => {
  test('rebinds a command, warns about a conflict, shows it in the cheat sheet, survives a reload and resets', async ({ page }, testInfo) => {
    await openApp(page);
    const dialog = await openEditor(page);
    await expectNoSeriousViolations(page, testInfo, 'shortcut-editor', 'debrief-keyboard-settings');

    await dialog.getByRole('searchbox', { name: 'Search shortcuts' }).fill('Toggle sidebar');
    const row = dialog.locator('tr[data-command="view.toggleSidebar"]');
    await expect(row).toBeVisible();
    await row.locator('button.key').click();
    await expect(row.locator('button.key')).toContainText('Press keys');
    await page.keyboard.press('Alt+J');
    await expect(row.locator('kbd').first()).toHaveText(ALT_J);
    await expect(dialog.getByTestId('shortcut-status')).not.toHaveText('');

    // A key that another command already uses asks before it is taken.
    await row.locator('button.key').click();
    await expect(row.locator('button.key')).toContainText('Press keys');
    await page.keyboard.press('ControlOrMeta+k');
    const conflict = dialog.getByTestId('shortcut-conflict');
    await expect(conflict).toBeVisible();
    await dialog.getByTestId('shortcut-cancel').click();
    await expect(conflict).toHaveCount(0);
    await expect(row.locator('kbd').first()).toHaveText(ALT_J);

    // Close Settings; the new key works, and the cheat sheet lists it.
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    const sidebar = page.locator('debrief-sidebar');
    const before = await sidebar.count();
    await page.keyboard.press('Alt+J');
    await expect(sidebar).toHaveCount(before === 0 ? 1 : 0);
    await page.keyboard.press('Alt+J');
    await expect(sidebar).toHaveCount(before);
    await page.keyboard.press('F1');
    const sheet = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
    await expect(sheet.locator('.cs-row').filter({ hasText: 'Toggle sidebar' }).locator('kbd')).toHaveText(ALT_J);
    await expectNoSeriousViolations(page, testInfo, 'cheat-sheet-overrides', 'debrief-cheat-sheet');
    await page.keyboard.press('Escape');

    // Kept across a reload; Reset puts the default back.
    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await chooseMenu(page, 'Help', 'Keyboard shortcuts');
    await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' }).locator('.cs-row').filter({ hasText: 'Toggle sidebar' }).locator('kbd')).toHaveText(ALT_J);
    await page.keyboard.press('Escape');
    const again = await openEditor(page);
    await again.getByRole('searchbox', { name: 'Search shortcuts' }).fill('Toggle sidebar');
    await again.getByRole('button', { name: /^Reset/ }).first().click();
    await expect(again.locator('tr[data-command="view.toggleSidebar"] kbd').first()).not.toHaveText(ALT_J);
  });

  test('the cheat sheet links to the editor', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('F1');
    await page.getByTestId('cheat-sheet-edit').click();
    await expect(page.getByRole('dialog', { name: 'Settings' }).getByRole('searchbox', { name: 'Search shortcuts' })).toBeVisible();
  });
});
