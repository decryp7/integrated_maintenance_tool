import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

test('Plugins panel lists installed plugins and built-ins', async ({ page }, testInfo) => {
  await openApp(page);
  await page.getByRole('button', { name: 'View', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'Plugins' }).click();
  const panel = page.locator('imt-plugins-view');
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('list', { name: 'Installed plugins' }).getByRole('listitem')).toHaveCount(4);
  await expect(panel).toContainText('Windows registry importer');
  await expect(panel).toContainText('About plugins');
  await expectNoSeriousViolations(page, testInfo, 'plugins');
});
