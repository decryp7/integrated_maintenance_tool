import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test('Plugins panel lists installed plugins and built-ins', async ({ page }, testInfo) => {
  await openApp(page);
  await chooseMenu(page, 'View', 'Plugins', 'Manage plugins');
  const panel = page.locator('debrief-plugins-view');
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('list', { name: 'Installed plugins' }).getByRole('listitem')).toHaveCount(4);
  await expect(panel).toContainText('Windows registry importer');
  await expect(panel).toContainText('About plugins');
  await expectNoSeriousViolations(page, testInfo, 'plugins');
});
