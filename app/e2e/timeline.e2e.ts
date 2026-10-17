import { expect, Locator, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';

/** The button in the filter bar that shows (and resets) the visible time range. */
const rangeButton = (page: Page) => page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/);

/** Drags across the histogram strip (no markers there) between two fractions of the plot width. */
async function brush(page: Page, from: number, to: number): Promise<void> {
  const box = (await page.locator('imt-timeline-panel .hist-row .plot').boundingBox())!;
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * from, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * ((from + to) / 2), y, { steps: 5 });
  await page.mouse.move(box.x + box.width * to, y, { steps: 5 });
  await page.mouse.up();
}

async function rangeText(loc: Locator): Promise<string> {
  return (await loc.innerText()).trim();
}

test.describe('timeline zoom', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('brush-drag zooms and Back restores the range', async ({ page }) => {
    const range = rangeButton(page);
    const full = await rangeText(range);
    const fit = page.getByRole('button', { name: 'Fit' });
    await expect(fit).toBeDisabled();
    const all = await initialTotal(page);

    await brush(page, 0.3, 0.5);
    await expect(range).not.toHaveText(full);
    await expect(range).toContainText('(');
    await expect(fit).toBeEnabled();
    await expect(gridTotal(page)).not.toHaveText(all);

    await page.getByRole('button', { name: 'Previous range' }).click();
    await expect(range).toHaveText(full);
    await expect(gridTotal(page)).toHaveText(all);
  });

  test('Fit resets a zoomed range', async ({ page }) => {
    const range = rangeButton(page);
    const full = await rangeText(range);
    await brush(page, 0.2, 0.4);
    await expect(range).not.toHaveText(full);
    await page.getByRole('button', { name: 'Fit' }).click();
    await expect(range).toHaveText(full);
    await expect(page.getByRole('button', { name: 'Fit' })).toBeDisabled();
  });

  test('double-click resets the range', async ({ page }) => {
    const range = rangeButton(page);
    const full = await rangeText(range);
    await brush(page, 0.4, 0.6);
    await expect(range).not.toHaveText(full);
    const box = (await page.locator('imt-timeline-panel .hist-row .plot').boundingBox())!;
    await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2);
    await expect(range).toHaveText(full);
  });

  test('Ctrl+wheel zooms around the cursor', async ({ page }) => {
    const range = rangeButton(page);
    const full = await rangeText(range);
    const box = (await page.locator('imt-timeline-panel .hist-row .plot').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.down('Control');
    await page.mouse.wheel(0, -400);
    await page.keyboard.up('Control');
    await expect(range).not.toHaveText(full);
    await expect(range).toContainText('(');
  });
});
