import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

test('settings dialog changes theme, date format and density, and persists them', async ({ page, browserName }, testInfo) => {
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const dialog = page.getByRole('dialog', { name: 'Settings' });
  await expect(dialog).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'settings-dialog');

  await dialog.getByRole('button', { name: 'light' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await dialog.getByRole('button', { name: 'ISO' }).click();
  await dialog.getByRole('button', { name: 'compact' }).click();
  await expect(page.locator('imt-events-grid')).toHaveCSS('--row-h', '22px');
  await expect(page.locator('imt-events-grid .row .time').first()).toHaveText(/^\d{4}-\d{2}-\d{2} /);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  // Safari does not focus clicked buttons, so there is nothing to restore focus to there.
  if (browserName === 'chromium') await expect(page.getByRole('button', { name: 'Settings' })).toBeFocused();

  await page.reload();
  await expect(page.locator('imt-events-grid .row .time').first()).toHaveText(/^\d{4}-\d{2}-\d{2} /);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
});

test('settings can reset the layout', async ({ page }) => {
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('dialog', { name: 'Settings' }).getByRole('button', { name: 'Reset layout' }).click();
  await expect(page.getByRole('dialog', { name: 'Settings' })).toHaveCount(0);
  await expect(page.getByRole('tab', { name: 'Timeline', selected: true })).toBeVisible();
});
