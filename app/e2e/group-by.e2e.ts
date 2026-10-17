import { expect, test } from '@playwright/test';
import { openApp } from './app';

test('changing group-by alters the lane labels', async ({ page }) => {
  await openApp(page);
  const lanes = page.locator('imt-timeline-panel .lane-label .key');
  await expect(lanes).toHaveCount(7);
  await expect(lanes.first()).toContainText(/\d{3,4}$/); // ends in a ProcessId

  await page.getByRole('button', { name: /Group by/ }).click();
  await page.getByRole('button', { name: 'Remove ProcessId' }).click();
  await expect(lanes).toHaveCount(5);
  await expect(lanes.filter({ hasText: /\d{3,4}$/ })).toHaveCount(0);
  await expect(page.locator('imt-status-bar')).toContainText('Group: Application, MachineName');

  await page.getByRole('button', { name: 'Use profile default' }).click();
  await expect(lanes).toHaveCount(7);
});
