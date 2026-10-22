import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView } from './tools-support';

test('Plugin health lists plugins and offers Restart to apply after a change', async ({ page }, testInfo) => {
  await openAppWith(page);
  await openView(page, 'Plugins');
  await page.getByTestId('plugins-tab-health').click();
  const panel = page.locator('imt-plugin-health-view');
  const items = panel.getByRole('list', { name: 'Plugin health' }).locator(':scope > li');
  await expect(items).toHaveCount(4);
  await expect(items.first().getByTestId('plugin-status')).toHaveText('active');
  await expectNoSeriousViolations(page, testInfo, 'plugin-health');
  await items.first().getByRole('button', { name: 'Disable' }).click();
  await expect(items.first()).toContainText('will be disabled');
  await expect(panel.getByRole('button', { name: 'Restart to apply' })).toBeVisible();
  await items.first().getByRole('button', { name: 'Enable' }).click();
  await expect(items.first()).toContainText('will be enabled');
});

test('Plugin health shows dependencies, why a plugin is inactive and event-bus counters', async ({ page }) => {
  await openAppWith(page);
  await openView(page, 'Plugins');
  await page.getByTestId('plugins-tab-health').click();
  const panel = page.locator('imt-plugin-health-view');
  const otel = panel.locator('.plugin[data-status=inactive]');
  await expect(otel.getByTestId('inactive-reason')).toContainText('needs net.decryptology.collector');
  const web = panel.getByRole('listitem').filter({ hasText: 'Web server logs' }).first();
  await expect(web).toContainText('net.decryptology.perfmon ^1.0 (optional)');
  await expect(web.getByRole('list', { name: 'Warnings' })).toContainText('perfmon is not installed');
  await expect(panel.getByRole('table', { name: 'Events delivered to Windows event logs' })).toContainText('RecordsAppended');
  await panel.getByRole('button', { name: 'Refresh event counters' }).click();
  await expect(panel.getByRole('table', { name: 'Events delivered to Web server logs' })).toContainText('ProfileChanged');
});
