import { expect, test } from '@playwright/test';
import { openApp } from './app';

test('metrics tab renders chart cards with a canvas each', async ({ page }) => {
  await openApp(page);
  await page.getByRole('tab', { name: 'Metrics' }).click();
  await expect(page.getByRole('tab', { name: 'Metrics', selected: true })).toBeVisible();

  const cards = page.locator('imt-chart-card');
  await expect(cards.first()).toBeVisible();
  const n = await cards.count();
  expect(n).toBeGreaterThanOrEqual(2);
  for (let i = 0; i < n; i++) {
    await expect(cards.nth(i).locator('canvas').first()).toBeVisible();
    await expect(cards.nth(i).locator('.title')).not.toBeEmpty();
  }
});
