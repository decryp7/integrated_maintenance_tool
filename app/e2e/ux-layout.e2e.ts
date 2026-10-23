import { expect, Page, test } from '@playwright/test';
import { ensureInspector, openApp } from './app';

// Usability review 2026-10-23 (U1, U2): the events grid is the main artefact, so the default layout, a document opened
// beside the timeline and a pinned Inspector all leave it many rows, and the timeline plot keeps its width.

/** Rows of the events grid that are fully inside its viewport. */
async function visibleGridRows(page: Page): Promise<number> {
  return page.evaluate(() => {
    const grid = document.querySelector('debrief-events-grid')!.getBoundingClientRect();
    return [...document.querySelectorAll('debrief-events-grid [role=row].row')].filter((e) => {
      const b = e.getBoundingClientRect();
      return b.height > 0 && b.top >= grid.top && b.bottom <= grid.bottom + 1;
    }).length;
  });
}

const laneWidth = (page: Page) => page.locator('debrief-timeline-panel .lane').first().evaluate((e) => Math.round(e.getBoundingClientRect().width));

const SIZES = [
  { width: 1440, height: 900, rows: 12, plot: 600 },
  { width: 1024, height: 768, rows: 9, plot: 480 },
  { width: 2560, height: 1300, rows: 20, plot: 1200 },
];

for (const size of SIZES) {
  test.describe(`${size.width}x${size.height}`, () => {
    test.use({ viewport: { width: size.width, height: size.height } });

    test('the default layout gives the grid its rows and the timeline its width', async ({ page }) => {
      await page.addInitScript(() => localStorage.clear());
      await openApp(page);
      expect(await visibleGridRows(page)).toBeGreaterThanOrEqual(size.rows);
      expect(await laneWidth(page)).toBeGreaterThanOrEqual(size.plot);
    });

    test('pinning the Inspector keeps the grid rows', async ({ page }) => {
      await page.addInitScript(() => localStorage.clear());
      await openApp(page);
      await ensureInspector(page);
      await expect.poll(() => visibleGridRows(page)).toBeGreaterThanOrEqual(size.rows);
    });
  });
}

// U4: the auto-hide Inspector slides out over the right edge; the selected row (its Message column) stays clear of it.
test('the Inspector flyout does not cover the selected row', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  await page.locator('debrief-events-grid [role=row].row').nth(3).click();
  const flyout = page.locator('#debrief-ah-flyout');
  await expect(flyout).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const row = document.querySelector('debrief-events-grid [role=row].row[aria-selected=true]');
        const fly = document.querySelector('#debrief-ah-flyout');
        return row && fly ? Math.round(fly.getBoundingClientRect().left - row.getBoundingClientRect().right) : null;
      }),
    )
    .toBeGreaterThanOrEqual(-1);
  // Closing it gives the grid its width back.
  await page.keyboard.press('Escape');
  await expect(flyout).toHaveCount(0);
  await expect.poll(() => page.locator('debrief-events-grid').evaluate((e) => (e as HTMLElement).style.getPropertyValue('--debrief-flyout-inset-right'))).toBe('');
});

// U6: Compare opens with sensible windows (not an empty one before the data) and does not starve the grid.
test('Compare time ranges starts from the data and leaves the grid its rows', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  const before = await visibleGridRows(page);
  await page.keyboard.press('ControlOrMeta+k');
  await page.keyboard.type('Compare time ranges');
  await page.keyboard.press('Enter');
  const compare = page.locator('debrief-compare-view');
  await expect(compare.locator('.card .nums').first()).toBeVisible({ timeout: 10_000 });
  const events = await compare.locator('.card .nums').allInnerTexts();
  expect(events).toHaveLength(2);
  for (const text of events) expect(text).not.toMatch(/^0 events|^1 events/);
  await expect.poll(() => visibleGridRows(page)).toBeGreaterThanOrEqual(7);
  expect(before).toBeGreaterThanOrEqual(12);
});

// U12: the filter bar is one row at 1440 (the lens and the view options no longer wrap onto a second, mostly empty row).
test('the filter bar is a single row at 1440', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  const h = await page.locator('debrief-filter-bar').evaluate((e) => Math.round(e.getBoundingClientRect().height));
  expect(h).toBeLessThanOrEqual(60);
});
