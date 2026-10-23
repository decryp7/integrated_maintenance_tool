import { expect, test } from '@playwright/test';
import { openApp, openViewOptions } from './app';
import { expectNoSeriousViolations } from './axe';

test('changing group-by alters the lane labels', async ({ page }) => {
  await openApp(page);
  const lanes = page.locator('debrief-timeline-panel .lane-label .key');
  await expect(lanes).toHaveCount(7);
  await expect(lanes.first()).toContainText(/\d{3,4}$/); // ends in a ProcessId

  // Group by is under the filter bar's View options.
  await openViewOptions(page);
  await page.getByRole('button', { name: 'Remove ProcessId' }).click();
  await expect(lanes).toHaveCount(5);
  await expect(lanes.filter({ hasText: /\d{3,4}$/ })).toHaveCount(0);
  await expect(page.locator('debrief-status-bar')).toContainText('Group: Application, MachineName');

  await page.getByRole('button', { name: 'Use profile default' }).click();
  await expect(lanes).toHaveCount(7);
});

test('View options: a plain heading, and every group-by candidate is a 24 px target (axe target-size)', async ({ page }, testInfo) => {
  await openApp(page);
  await openViewOptions(page);
  const options = page.locator('#debrief-view-options');
  await expect(options).toContainText('Group events into timeline lanes by');
  await expect(options).not.toContainText('Instance key');
  const rows = options.getByRole('group', { name: 'Add a field to the group-by' }).getByRole('button');
  expect(await rows.count()).toBeGreaterThan(3);
  for (const h of await rows.evaluateAll((els) => els.map((e) => e.getBoundingClientRect().height))) expect(h).toBeGreaterThanOrEqual(24);
  await expectNoSeriousViolations(page, testInfo, 'view-options', '#debrief-view-options');
});
