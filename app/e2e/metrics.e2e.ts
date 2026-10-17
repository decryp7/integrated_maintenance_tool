import { expect, test } from '@playwright/test';
import { openApp } from './app';

test('metrics tab renders chart cards with a canvas each', async ({ page }) => {
  await openApp(page);
  await page.getByRole('tab', { name: 'Metrics' }).click();
  await expect(page.getByRole('tab', { name: 'Metrics', selected: true })).toBeVisible();

  const cards = page.locator('imt-chart-card');
  await expect.poll(() => cards.count()).toBeGreaterThanOrEqual(2);
  const n = await cards.count();
  for (let i = 0; i < n; i++) {
    await expect(cards.nth(i).locator('canvas').first()).toBeVisible();
    await expect(cards.nth(i).locator('.title')).not.toBeEmpty();
  }
});
