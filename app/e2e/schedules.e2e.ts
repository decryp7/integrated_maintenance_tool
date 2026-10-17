import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView } from './tools-support';

test.describe('Collection schedules and runs', () => {
  test('edits, validates and saves the agent configuration', async ({ page }, testInfo) => {
    await openAppWith(page);
    await openView(page, 'Collection schedules');
    const panel = page.locator('imt-schedules-view');
    await expect(panel.getByTestId('schedule-0')).toContainText('nightly');
    await expect(panel.getByTestId('schedules-state')).toHaveText('Saved');

    await panel.getByRole('button', { name: 'Add schedule' }).click();
    const added = panel.getByTestId('schedule-1');
    await added.getByRole('textbox', { name: 'Name' }).fill('memory-spike');
    await added.getByLabel('Runs').selectOption('trigger');
    await added.getByRole('textbox', { name: 'Process name' }).fill('OrderService*');
    await added.getByRole('combobox', { name: 'Kind of action 1' }).selectOption('monitor');
    await added.getByRole('textbox', { name: 'Processes' }).fill('OrderService*');
    await added.getByRole('textbox', { name: 'Duration' }).fill('soon');
    await expect(panel.getByTestId('schedules-state')).toHaveText('Unsaved changes');
    await expectNoSeriousViolations(page, testInfo, 'schedules-editor', 'imt-schedules-view');

    await panel.getByRole('button', { name: 'Validate' }).click();
    await expect(added.getByRole('alert')).toContainText('between 1s and 24h');
    await expect(panel.getByTestId('schedules-state')).toHaveText('1 problem');
    await panel.getByRole('button', { name: 'Save' }).click();
    await expect(panel.getByRole('alert').first()).toContainText('Not saved');

    await added.getByRole('textbox', { name: 'Duration' }).fill('10m');
    await panel.getByRole('button', { name: 'Save' }).click();
    await expect(panel.getByTestId('schedules-state')).toHaveText('Saved');
    await expect(panel.getByRole('status').first()).toContainText('Saved /demo/config/imt-agent.toml');
  });

  test('browses runs, shows a manifest and opens a run in a new window', async ({ page }, testInfo) => {
    await openAppWith(page);
    await openView(page, 'Collection runs');
    const panel = page.locator('imt-runs-view');
    const runs = panel.getByRole('table', { name: 'Runs' });
    await expect(runs.locator('tbody tr')).toHaveCount(3);
    await expect(panel).toContainText('/demo/agent/runs');
    await panel.getByRole('button', { name: 'Details of 20261016-143012-memory-spike' }).click();
    await expect(panel.getByTestId('run-manifest')).toContainText('only available on Windows');
    await expectNoSeriousViolations(page, testInfo, 'runs-browser', 'imt-runs-view');
    await panel.getByRole('button', { name: 'Open 20261017-020000-nightly in a new window' }).click();
    await expect(panel.getByRole('status')).toContainText('Opened 20261017-020000-nightly in a new window');
  });
});
