import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView } from './tools-support';

test('Plugin health lists plugins and offers Restart to apply after a change', async ({ page }, testInfo) => {
  await openAppWith(page);
  await openView(page, 'Plugin health');
  const panel = page.locator('imt-plugin-health-view');
  const items = panel.getByRole('list', { name: 'Plugin health' }).getByRole('listitem');
  await expect(items).toHaveCount(2);
  await expect(items.first().getByTestId('plugin-status')).toHaveText('active');
  await expectNoSeriousViolations(page, testInfo, 'plugin-health');
  await items.first().getByRole('button', { name: 'Disable' }).click();
  await expect(items.first()).toContainText('will be disabled');
  await expect(panel.getByRole('button', { name: 'Restart to apply' })).toBeVisible();
  await items.first().getByRole('button', { name: 'Enable' }).click();
  await expect(items.first()).toContainText('will be enabled');
});
