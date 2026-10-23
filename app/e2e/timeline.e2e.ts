import { expect, Locator, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';
import { openAppWith } from './mock-url';

/** The button in the filter bar that shows (and resets) the visible time range. */
const rangeButton = (page: Page) => page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/);

/** Drags across the histogram strip (no markers there) between two fractions of the plot width. */
async function brush(page: Page, from: number, to: number): Promise<void> {
  const box = (await page.locator('debrief-timeline-panel .hist-row .plot').boundingBox())!;
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
    const box = (await page.locator('debrief-timeline-panel .hist-row .plot').boundingBox())!;
    await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2);
    await expect(range).toHaveText(full);
  });

  test('Ctrl+wheel zooms around the cursor', async ({ page }) => {
    const range = rangeButton(page);
    const full = await rangeText(range);
    const box = (await page.locator('debrief-timeline-panel .hist-row .plot').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.down('Control');
    await page.mouse.wheel(0, -400);
    await page.keyboard.up('Control');
    await expect(range).not.toHaveText(full);
    await expect(range).toContainText('(');
  });
});

test.describe('timeline and analysis', () => {
  test('right-clicking the time axis offers "Analyse this moment"', async ({ page }) => {
    await openApp(page);
    const axis = page.locator('debrief-timeline-panel .axis');
    await axis.click({ button: 'right', position: { x: 200, y: 10 } });
    const menu = page.getByRole('menu', { name: 'Actions' });
    await expect(menu).toBeVisible();
    await menu.getByRole('menuitem', { name: 'Analyse this moment' }).click();
    await expect(menu).toHaveCount(0);
    const view = page.locator('debrief-moment-view');
    await expect(view).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Instances' })).toBeVisible();
  });

  test('hovering the timeline draws the shared time cursor', async ({ page }) => {
    await openApp(page);
    const box = (await page.locator('debrief-timeline-panel .hist-row .plot').boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await expect(page.locator('debrief-timeline-panel .cursor-label')).toHaveText(/\d{2}:\d{2}:\d{2}/);
    await page.mouse.move(box.x + box.width / 2, box.y - 200);
    await expect(page.locator('debrief-timeline-panel .cursor')).toBeHidden();
  });

  test('a failed timeline request shows an inline error with Retry', async ({ page }) => {
    // The panel asks again when its width is measured, so more than one request fails before Retry succeeds.
    await openAppWith(page, '?mockFail=queryTimeline:3&mockSnapshots=off');
    const card = page.locator('debrief-timeline-panel .err-card');
    await expect(card).toContainText("Couldn't load the timeline.");
    await expect(card).toContainText('Simulated queryTimeline failure');
    await expect(page.getByTestId('workspace-error')).toHaveCount(0);
    await expect(async () => {
      if (await card.count()) await card.getByRole('button', { name: 'Retry' }).click();
      await expect(card).toHaveCount(0, { timeout: 1500 });
    }).toPass({ timeout: 15_000 });
    await expect(page.locator('debrief-timeline-panel .lane').first()).toBeVisible();
  });
});
