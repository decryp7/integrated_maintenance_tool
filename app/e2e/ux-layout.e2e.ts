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
