import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView } from './tools-support';

test.describe('Performance Monitor wizard', () => {
  test('builds a Data Collector Set: processes, presets, XML preview, install, import', async ({ page }, testInfo) => {
    await openAppWith(page);
    await openView(page, 'Performance Monitor');
    const panel = page.locator('debrief-perfmon-view');
    await expect(panel.getByRole('checkbox', { name: 'Preset CPU' })).toBeChecked();

    await panel.getByRole('textbox', { name: 'Process name' }).fill('OrderService.exe');
    await panel.getByRole('button', { name: 'Add process' }).click();
    await expect(panel.locator('.chip')).toContainText('OrderService');
    await panel.getByRole('checkbox', { name: 'Preset Disk' }).check();
    await panel.getByRole('button', { name: 'Show counters of CPU' }).click();
    await expect(panel.getByText('What to look for:').first()).toBeVisible();

    await panel.getByRole('textbox', { name: 'Collector name' }).fill('Repro');
    await panel.getByRole('button', { name: 'Generate XML' }).click();
    const xml = panel.getByLabel('Generated XML');
    await expect(xml).toContainText('<DataCollectorSet>');
    await expect(xml).toContainText('<Name>Repro</Name>');
    await expect(xml).toContainText('Process(OrderService)');
    await expect(panel.getByText('logman import -n "Repro"')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'perfmon-wizard');

    await panel.getByRole('button', { name: /Install/ }).click();
    const card = panel.getByTestId('collector-status');
    await expect(card.getByTestId('collector-state')).toHaveText('running');
    await card.getByRole('button', { name: 'Stop' }).click();
    await expect(card.getByTestId('collector-state')).toHaveText('stopped');
    await card.getByRole('button', { name: 'Start' }).click();
    await expect(card.getByTestId('collector-state')).toHaveText('running');
    await card.getByRole('button', { name: 'Import results' }).click();
    await expect(panel.getByRole('status').filter({ hasText: 'with live tail' })).toBeVisible();
    await card.getByRole('button', { name: 'Delete' }).click();
    await expect(card).toHaveCount(0);
  });

  test('offers a restart as administrator when access is denied', async ({ page }) => {
    await openAppWith(page, '?mock-denied=1');
    await openView(page, 'Performance Monitor');
    const panel = page.locator('debrief-perfmon-view');
    await panel.getByRole('button', { name: /Install/ }).click();
    await expect(panel.getByRole('alert')).toContainText('Access is denied');
    await expect(panel.getByRole('button', { name: 'Restart as administrator' })).toBeVisible();
  });

  test('away from Windows it still generates the XML and the commands', async ({ page }) => {
    await openAppWith(page, '?mock-os=mac');
    await openView(page, 'Performance Monitor');
    const panel = page.locator('debrief-perfmon-view');
    await expect(panel).toContainText('can only be created on Windows');
    await expect(panel.getByRole('button', { name: /Install/ })).toHaveCount(0);
    await panel.getByRole('button', { name: 'Generate XML' }).click();
    await expect(panel.getByLabel('Generated XML')).toContainText('<DataCollectorSet>');
  });
});
