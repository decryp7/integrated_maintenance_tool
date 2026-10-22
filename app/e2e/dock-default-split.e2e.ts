import { expect, test } from '@playwright/test';
import { openApp } from './app';

// The default split is applied once the dock has its size, so the layout is the same the moment the app is ready and
// a moment later (it used to settle at 300/489 or 282/507 px depending on when the container was measured).
test('the default layout does not move after the app is ready', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  const heights = () => page.evaluate(() => [...document.querySelectorAll('.dv-groupview')].map((g) => Math.round(g.getBoundingClientRect().height)));
  const first = await heights();
  await page.waitForTimeout(1000);
  expect(await heights()).toEqual(first);
  // The grid keeps its share and the timeline row takes the rest.
  expect(first.length).toBeGreaterThanOrEqual(2);
  expect(Math.min(...first)).toBeGreaterThan(200);
});
