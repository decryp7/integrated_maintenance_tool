import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test.describe('keyboard cheat sheet', () => {
  test('F1 opens a searchable, printable list of the shortcuts', async ({ page }, testInfo) => {
    await openApp(page);
    await page.keyboard.press('F1');
    const dialog = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText('Show all commands');
    await expect(dialog).toContainText('Focus the search box');
    await expect(dialog.getByRole('button', { name: 'Print…' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'cheat-sheet', 'imt-cheat-sheet');

    const search = dialog.getByRole('searchbox', { name: 'Search shortcuts' });
    await search.fill('sidebar');
    await expect(dialog.locator('.cs-row')).toHaveCount(1);
    await expect(dialog.locator('.cs-row')).toContainText('Toggle sidebar');
    await search.fill('no such shortcut');
    await expect(dialog).toContainText('No shortcut matches');
    await dialog.getByRole('button', { name: 'Clear search' }).click();
    await expect(dialog.locator('.cs-row').first()).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    // Also in the Help menu.
    await chooseMenu(page, 'Help', 'Keyboard shortcuts');
    await expect(dialog).toBeVisible();
  });
});
