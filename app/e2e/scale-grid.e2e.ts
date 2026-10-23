import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { gridTotal } from './app';
import { eventId, gridRows, openAppWith, viewport } from './mock-url';

const BIG = 50_000_000;

test.describe('events grid with 50 million rows', () => {
  test.beforeEach(async ({ page }) => {
    await openAppWith(page, `?mockTotal=${BIG}`);
    await expect(gridTotal(page)).toHaveText('50,000,000');
    await expect(gridRows(page).first()).toContainText('Synthetic event 0');
  });

  test('renders a screenful of rows and a capped scroll height', async ({ page }) => {
    expect(await gridRows(page).count()).toBeLessThan(60);
    const spacer = await viewport(page).evaluate((el) => (el.lastElementChild as HTMLElement).offsetHeight);
    expect(spacer).toBeLessThanOrEqual(10_000_000);
    expect(spacer).toBeGreaterThan(1_000_000);
  });

  test('the scrollbar reaches the very last row', async ({ page }) => {
    await viewport(page).evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    const last = gridRows(page).last();
    await expect(last).toContainText('Synthetic event 49999999');
    // The last row ends exactly at the bottom of the rows area.
    const vp = (await viewport(page).boundingBox())!;
    const box = (await last.boundingBox())!;
    expect(Math.abs(box.y + box.height - (vp.y + vp.height))).toBeLessThanOrEqual(2);
  });

  test('the scrollbar thumb at the middle shows rows from the middle', async ({ page }) => {
    await viewport(page).evaluate((el) => {
      el.scrollTop = (el.scrollHeight - el.clientHeight) / 2;
    });
    await expect.poll(async () => eventId(await gridRows(page).nth(12).textContent())).toBeGreaterThan(BIG / 2 - 1000);
    expect(eventId(await gridRows(page).nth(12).textContent())).toBeLessThan(BIG / 2 + 1000);
  });

  test('the mouse wheel moves a few rows at a time, not thousands', async ({ page }) => {
    await viewport(page).hover();
    await page.mouse.wheel(0, 260);
    await expect(async () => {
      const first = eventId(await gridRows(page).first().textContent());
      expect(first).toBeLessThanOrEqual(0 + 10);
    }).toPass();
    await expect(page.locator('debrief-events-grid .row', { hasText: 'Synthetic event 10' }).first()).toBeAttached();
    // About a hundred rows further (the row height comes from the theme and density).
    const rowH = await page.locator('debrief-events-grid').evaluate((el) => parseFloat(getComputedStyle(el).getPropertyValue('--row-h')));
    await page.mouse.wheel(0, 110 * rowH - 260);
    await expect(page.locator('debrief-events-grid .row', { hasText: 'Synthetic event 110' }).first()).toBeAttached();
  });

  test('keyboard: End, Home, arrows and pages move the selection and keep it on screen', async ({ page }) => {
    const sel = page.locator('debrief-events-grid .row.sel');
    await viewport(page).focus();
    await page.keyboard.press('End');
    await expect(sel).toContainText('Synthetic event 49999999');
    await expect(page.locator('debrief-inspector')).toContainText('#49999999');
    await page.keyboard.press('ArrowUp');
    await expect(sel).toContainText('Synthetic event 49999998');
    await page.keyboard.press('PageUp');
    await expect.poll(async () => eventId(await sel.textContent())).toBeLessThan(49999998 - 5);
    await page.keyboard.press('Home');
    await expect(sel).toContainText('Synthetic event 0');
    await page.keyboard.press('ArrowDown');
    await expect(sel).toContainText('Synthetic event 1');
    for (let i = 0; i < 60; i++) await page.keyboard.press('ArrowDown');
    await expect(sel).toContainText('Synthetic event 61');
    const vp = (await viewport(page).boundingBox())!;
    const box = (await sel.boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(vp.y);
    expect(box.y + box.height).toBeLessThanOrEqual(vp.y + vp.height + 1);
  });

  test('jumping to an event near 40 million lands on it exactly', async ({ page }) => {
    // Park the scrollbar at 80% and pick a row there, then "Show ±2 min" jumps to that event.
    await viewport(page).evaluate((el) => {
      el.scrollTop = (el.scrollHeight - el.clientHeight) * 0.8;
    });
    await expect.poll(async () => eventId(await gridRows(page).nth(10).textContent())).toBeGreaterThan(39_000_000);
    const row = gridRows(page).nth(10);
    const id = eventId(await row.textContent());
    await row.click();
    await page.getByRole('button', { name: 'Show ±2 min' }).click();
    const sel = page.locator('debrief-events-grid .row.sel');
    await expect(sel).toContainText(`Synthetic event ${id}`);
    // The query changes first (rows reload), then the jump centres the row: poll until it has settled.
    await expect
      .poll(
        async () => {
          const vp = await viewport(page).boundingBox();
          const box = await sel.boundingBox();
          if (!vp || !box) return Number.POSITIVE_INFINITY;
          const header = 28;
          return Math.abs(box.y + box.height / 2 - (vp.y + header + (vp.height - header) / 2));
        },
        { timeout: 10_000 },
      )
      .toBeLessThanOrEqual(30);
  });

  test('is accessible', async ({ page }, testInfo) => {
    await viewport(page).evaluate((el) => {
      el.scrollTop = el.scrollHeight / 3;
    });
    await expect(gridRows(page).first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'scale-grid', 'debrief-events-grid');
  });
});
