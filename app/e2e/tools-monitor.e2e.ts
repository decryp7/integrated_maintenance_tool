import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView, sourceCount } from './tools-support';

test.describe('Process monitor', () => {
  test('starts with live tail on, shows the folder, and stops', async ({ page }, testInfo) => {
    await openAppWith(page);
    const before = await sourceCount(page);
    await openView(page, 'Process Monitor');
    const panel = page.locator('debrief-monitor-view');
    const status = panel.getByTestId('monitor-status');
    await expect(status).toContainText('Stopped');
    await expect(panel.getByRole('button', { name: 'Start' })).toBeDisabled();

    await panel.getByLabel('Target value 1').fill('OrderService*');
    await panel.getByRole('button', { name: /Add target/ }).click();
    await panel.getByLabel('Target type 2').click();
    await page.getByRole('option', { name: 'Process id' }).click();
    await panel.getByLabel('Target value 2').fill('1234');
    await panel.getByRole('spinbutton').fill('1000');
    await expectNoSeriousViolations(page, testInfo, 'process-monitor');

    await panel.getByRole('button', { name: 'Start' }).click();
    await expect(status).toContainText('Running');
    await expect(status).toContainText('/demo/monitor/run-1');
    // The output folder was opened in the workspace.
    await expect.poll(() => sourceCount(page)).toBe(before + 1);
    await panel.getByRole('button', { name: 'Stop' }).click();
    await expect(status).toContainText('Stopped');
  });
});
