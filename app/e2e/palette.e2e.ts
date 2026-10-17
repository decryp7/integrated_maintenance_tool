import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

test.describe('command palette', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('opens with Ctrl+K and from the title bar, filters, navigates and runs a command', async ({ page }, testInfo) => {
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await expect(input).toBeFocused();
    await expectNoSeriousViolations(page, testInfo, 'palette');

    await input.fill('local');
    const options = page.getByRole('option');
    await expect(options.first()).toContainText('Time in local zone');
    await page.keyboard.press('Escape');
    await expect(input).toHaveCount(0);

    await page.locator('.palette').click();
    await expect(input).toBeFocused();
    await input.fill('toggle utc');
    await expect(options.first()).toContainText('Toggle UTC / local time');
    await page.keyboard.press('Enter');
    await expect(input).toHaveCount(0);
    await expect(page.locator('imt-status-bar .tz')).not.toHaveText('UTC');
  });

  test('arrow keys move the highlight and Enter runs it', async ({ page }) => {
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await input.fill('theme');
    const options = page.getByRole('option');
    // Other commands (e.g. from plugins) may match too; the three theme commands must be there.
    for (const name of ['Theme: system', 'Theme: dark', 'Theme: light']) await expect(page.getByRole('option', { name: new RegExp(name) })).toHaveCount(1);
    await expect(options.first()).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowDown');
    await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowUp');
    await page.keyboard.press('ArrowUp');
    // Up from the first option wraps to the last one.
    await expect(options.last()).toHaveAttribute('aria-selected', 'true');
    await expect(page.locator('[role=option][aria-selected=true]')).toHaveCount(1);
    await input.fill('theme: light');
    await page.keyboard.press('Enter');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  });

  test('shows key hints and plugin commands, and says when nothing matches', async ({ page }) => {
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await input.fill('registry');
    await expect(page.getByRole('option', { name: /Show registry viewer/ })).toBeVisible();
    await input.fill('export');
    await expect(page.getByRole('option', { name: /Export events/ }).locator('kbd')).toBeVisible();
    await input.fill('zzzzqq');
    await expect(page.getByText('No matching commands')).toBeVisible();
  });
});
