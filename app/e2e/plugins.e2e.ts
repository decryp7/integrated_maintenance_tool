import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test('Plugins panel lists installed plugins and built-ins', async ({ page }, testInfo) => {
  await openApp(page);
  await chooseMenu(page, 'View', 'Plugins', 'Manage plugins');
  const panel = page.locator('imt-plugins-view');
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('table', { name: 'Installed plugins' }).locator('tr.plugin')).toHaveCount(4);
  await expect(panel).toContainText('Windows registry importer');
  await expect(panel).toContainText('About plugins');
  await expectNoSeriousViolations(page, testInfo, 'plugins', 'imt-plugins-view');
});

test('a plugin row opens its side panel with capabilities and the memory cap', async ({ page }) => {
  await openApp(page);
  await chooseMenu(page, 'View', 'Plugins', 'Manage plugins');
  const panel = page.locator('imt-plugins-view');
  await panel.locator('tr.plugin', { hasText: 'Windows event logs' }).click();
  const detail = panel.getByTestId('plugin-detail');
  await expect(detail).toContainText('importer:evtx');
  await detail.getByTestId('plugin-memory-input').fill('768');
  await detail.getByTestId('plugin-memory-input').blur();
  await expect(detail).toContainText('Use the default');
  await detail.getByRole('button', { name: 'Close details' }).click();
  await expect(detail).toHaveCount(0);
});
