import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';
import { expectNoSeriousViolations } from './axe';

const rangeButton = (page: Page) => page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/);
const strip = (page: Page) => page.getByTestId('timeline-minimap');
const win = (page: Page) => page.getByTestId('minimap-window');

/** Drags across the histogram strip between two fractions of the plot width (zooms the timeline). */
async function brush(page: Page, from: number, to: number): Promise<void> {
  const box = (await page.locator('imt-timeline-panel .hist-row .plot').boundingBox())!;
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * from, y);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * to, y, { steps: 8 });
  await page.mouse.up();
}

test.describe('timeline minimap', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('shows the whole extent with a window for the visible range, and is accessible', async ({ page }, testInfo) => {
    await expect(strip(page)).toBeVisible();
    await expect(strip(page).locator('svg.mini path').first()).toBeAttached();
    // Fully zoomed out: the window covers the strip.
    const box = (await strip(page).boundingBox())!;
    const w = (await win(page).boundingBox())!;
    expect(w.width).toBeGreaterThan(box.width * 0.95);
    await brush(page, 0.3, 0.5);
    await expect.poll(async () => (await win(page).boundingBox())!.width).toBeLessThan(box.width * 0.4);
    await expect(win(page)).toHaveAttribute('aria-valuetext', /to/);
    await expectNoSeriousViolations(page, testInfo, 'timeline-minimap', '[data-testid=timeline-minimap]');
  });

  test('dragging the window pans, clicking the strip centres it, double-click fits', async ({ page }) => {
    const all = await initialTotal(page);
    await brush(page, 0.2, 0.4);
    await expect(gridTotal(page)).not.toHaveText(all);
    const range = rangeButton(page);
    const zoomed = (await range.innerText()).trim();
    const box = (await strip(page).boundingBox())!;
    const y = box.y + box.height / 2;

    // Drag the window to the right: the range changes (applied on release).
    const w0 = (await win(page).boundingBox())!;
    await page.mouse.move(w0.x + w0.width / 2, y);
    await page.mouse.down();
    await page.mouse.move(w0.x + w0.width / 2 + box.width * 0.3, y, { steps: 8 });
    await page.mouse.up();
    await expect(range).not.toHaveText(zoomed);
    const w1 = (await win(page).boundingBox())!;
    expect(w1.x).toBeGreaterThan(w0.x + box.width * 0.2);
    expect(Math.abs(w1.width - w0.width)).toBeLessThan(4);

    // Click on the strip left of the window: the window centres there.
    await page.mouse.click(box.x + box.width * 0.1, y);
    await expect.poll(async () => {
      const w = (await win(page).boundingBox())!;
      return Math.abs(w.x + w.width / 2 - (box.x + box.width * 0.1));
    }).toBeLessThan(box.width * 0.04 + w1.width / 2);

    // Double-click resets to the whole extent.
    await page.mouse.dblclick(box.x + box.width * 0.6, y);
    await expect(page.getByRole('button', { name: 'Fit' })).toBeDisabled();
    await expect(gridTotal(page)).toHaveText(all);
  });

  test('the window is a slider: arrow keys pan the zoomed range', async ({ page }) => {
    await brush(page, 0.2, 0.4);
    const range = rangeButton(page);
    const before = (await range.innerText()).trim();
    await win(page).focus();
    await page.keyboard.press('ArrowRight');
    await expect(range).not.toHaveText(before);
  });
});
