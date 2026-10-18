import { expect, Locator, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView, sourceCount } from './tools-support';

/** The number of events the progress card shows. */
async function events(panel: Locator): Promise<number> {
  return Number((await panel.getByTestId('eventlog-events').innerText()).replace(/,/g, ''));
}

test.describe('Event Log follower', () => {
  test('picks channels, validates, starts with live tail, updates the status and stops', async ({ page }, testInfo) => {
    await openAppWith(page);
    const before = await sourceCount(page);
    await openView(page, 'Event Log Follower');
    const panel = page.locator('imt-eventlog-view');
    const status = panel.getByTestId('eventlog-status');
    await expect(status).toContainText('Stopped');
    await expect(panel.getByRole('button', { name: 'Start', exact: true })).toBeDisabled();

    await panel.getByRole('searchbox', { name: 'Search channels' }).fill('sys');
    await panel.getByRole('checkbox', { name: 'System' }).check();
    await panel.getByRole('searchbox', { name: 'Search channels' }).fill('');
    await panel.getByRole('checkbox', { name: 'ForwardedEvents' }).check();
    await expect(panel.getByTestId('eventlog-selected')).toHaveText('2 selected');
    await panel.getByRole('combobox', { name: 'Read back first' }).selectOption('1h');

    await panel.getByRole('button', { name: 'Validate' }).click();
    const validation = panel.getByTestId('eventlog-validation');
    await expect(validation).toContainText('Live query: *[System[(Level=1 or Level=2 or Level=3 or Level=0 or Level=4)]]');
    await expect(validation).toContainText('Backfill query:');
    await expect(validation).toContainText('System OK');
    await expect(validation).toContainText('ForwardedEvents The channel is disabled');
    await expectNoSeriousViolations(page, testInfo, 'event-log-follower');

    await panel.getByRole('checkbox', { name: 'ForwardedEvents' }).uncheck();
    await expect(validation).toHaveCount(0);
    await panel.getByRole('button', { name: 'Start', exact: true }).click();
    await expect(status).toContainText('Running');
    await expect(status).toContainText('/demo/eventlog/run-1');
    // The output folder was opened in the workspace.
    await expect.poll(() => sourceCount(page)).toBe(before + 1);
    // The status is polled while the run goes.
    const first = await events(panel);
    await expect.poll(() => events(panel)).toBeGreaterThan(first);
    await expect(panel.getByTestId('eventlog-progress')).toContainText('Live');
    await expect(panel.getByTestId('eventlog-last')).toContainText('System');

    await panel.getByRole('button', { name: 'Pause' }).click();
    await expect(status).toContainText('Paused');
    await panel.getByRole('button', { name: 'Resume' }).click();
    await expect(status).toContainText('Running');
    await panel.getByRole('button', { name: 'Stop' }).click();
    await expect(status).toContainText('Stopped');
  });

  test('Security without administrator rights offers Restart as administrator', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Event Log Follower');
    const panel = page.locator('imt-eventlog-view');
    await panel.getByRole('checkbox', { name: 'Security' }).check();
    await expect(panel.getByTestId('eventlog-admin-note')).toContainText('Security needs administrator rights');

    await panel.getByRole('button', { name: 'Start', exact: true }).click();
    const alert = panel.getByRole('alert');
    await expect(alert).toContainText('Access is denied');
    await expect(alert.getByRole('button', { name: 'Restart as administrator' })).toBeVisible();
    await expect(panel.getByTestId('eventlog-status')).toContainText('Stopped');
  });
});
