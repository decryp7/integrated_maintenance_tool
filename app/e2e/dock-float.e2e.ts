import { expect, Page, test } from '@playwright/test';
import { openApp, openView } from './app';

// Dragging a tab off the dock's drop zones shows where it will go: a ghost of the floating panel with "Release to
// float" inside the window, "Release to open in a new window" outside it. Tabs are dragged with pointer events.

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const cue = (page: Page) => page.locator('.imt-float-cue');
const floating = (page: Page) => page.locator('.dv-resize-container');

/** A point over the events grid well away from its compass (the middle) and the dock's edge guides. */
async function freeSpot(page: Page): Promise<{ x: number; y: number }> {
  const c = (await page.locator('.dv-groupview:has(imt-events-grid) .dv-content-container').boundingBox())!;
  return { x: c.x + c.width * 0.3, y: c.y + c.height * 0.3 };
}

async function pressAndDrag(page: Page, name: string): Promise<void> {
  const s = (await tab(page, name).boundingBox())!;
  await page.mouse.move(s.x + s.width / 2, s.y + s.height / 2);
  await page.mouse.down();
  await page.mouse.move(s.x + s.width / 2 + 20, s.y + s.height / 2 + 20, { steps: 5 });
}

test.describe('dragging a panel out to float', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
    await openView(page, 'metrics');
  });

  test('a ghost and hint show where the panel goes, clear over a drop zone, and the panel floats on release', async ({ page }) => {
    await pressAndDrag(page, 'Metrics');
    const free = await freeSpot(page);
    await page.mouse.move(free.x, free.y, { steps: 10 });
    await expect(cue(page)).toBeVisible();
    await expect(cue(page)).toHaveText('Release to float');
    await expect(page.getByRole('status').filter({ hasText: 'Release to float' })).toHaveCount(1);
    // The ghost is under the pointer.
    const g = (await cue(page).boundingBox())!;
    expect(free.x).toBeGreaterThanOrEqual(g.x);
    expect(free.x).toBeLessThanOrEqual(g.x + g.width);
    expect(free.y).toBeGreaterThanOrEqual(g.y);
    // Over the right guide of the events group's compass the dock takes it: no cue.
    const c = (await page.locator('.dv-groupview:has(imt-events-grid) .dv-content-container').boundingBox())!;
    await page.mouse.move(c.x + c.width / 2 + 36, c.y + c.height / 2, { steps: 10 });
    await expect(cue(page)).toBeHidden();
    // Outside the window it would open in a new window.
    await page.mouse.move(free.x, -40, { steps: 10 });
    await expect(cue(page)).toHaveText('Release to open in a new window');
    await page.mouse.move(free.x, free.y, { steps: 10 });
    await expect(cue(page)).toHaveText('Release to float');
    await page.mouse.up();
    await expect(cue(page)).toBeHidden();
    await expect(floating(page)).toBeVisible();
    await expect(floating(page).getByRole('tab', { name: 'Metrics', exact: true })).toBeVisible();
  });

  test('Escape cancels the drag: the cue clears and nothing floats', async ({ page }) => {
    await pressAndDrag(page, 'Metrics');
    const free = await freeSpot(page);
    await page.mouse.move(free.x, free.y, { steps: 10 });
    await expect(cue(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(cue(page)).toBeHidden();
    await page.mouse.up();
    await expect(floating(page)).toHaveCount(0);
    await expect(tab(page, 'Metrics')).toBeVisible();
  });
});
