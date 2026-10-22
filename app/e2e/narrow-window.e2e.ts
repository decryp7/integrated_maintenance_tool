import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// Narrow windows: the filter bar folds range, time zone and Clear all into a ⋯ menu (bar < 1000 px), the Explorer
// closes by itself below 1100 px (and comes back), low-priority status items go below 1000 px, and the page never
// scrolls sideways.

const sidebar = (page: Page) => page.locator('imt-sidebar');
const more = (page: Page) => page.getByTestId('filter-more');
const noHorizontalScroll = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);

async function resize(page: Page, width: number): Promise<void> {
  await page.setViewportSize({ width, height: 800 });
}

test.describe('narrow window', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
  });

  test('wide: the Explorer is open, nothing is folded, the status bar shows everything', async ({ page }) => {
    await expect(sidebar(page)).toBeVisible();
    await expect(more(page)).toHaveCount(0);
    await expect(page.locator('imt-status-bar .low').first()).toBeVisible();
  });

  test('1024 px: the Explorer has closed by itself, the page does not scroll sideways', async ({ page }, testInfo) => {
    await resize(page, 1024);
    await expect(sidebar(page)).toHaveCount(0);
    expect(await noHorizontalScroll(page)).toBe(true);
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'narrow-1024');
  });

  test('900 px: the filter bar folds into the ⋯ menu, which has range, time zone and Clear all', async ({ page }, testInfo) => {
    await resize(page, 900);
    await expect(sidebar(page)).toHaveCount(0);
    await expect(more(page)).toBeVisible();
    await expect(page.locator('imt-filter-bar .tz.fold')).toBeHidden();
    expect(await noHorizontalScroll(page)).toBe(true);
    // The search and the level chips always stay.
    await expect(page.getByTestId('query-field')).toBeVisible();
    await expect(page.getByRole('toolbar', { name: 'Levels' })).toBeVisible();
    await expect(page.locator('imt-status-bar .low').first()).toBeHidden();

    await page.getByRole('toolbar', { name: 'Levels' }).getByRole('button', { name: /^ERR/ }).click();
    await more(page).click();
    const menu = page.getByRole('menu', { name: 'More filter controls' });
    await expect(menu.getByRole('menuitem', { name: /Time zone/ })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: 'Clear all' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'narrow-900-more', 'imt-filter-bar');
    await menu.getByRole('menuitem', { name: /Time zone/ }).click();
    await expect(menu).toHaveCount(0);
    await more(page).click();
    await page.getByRole('menu', { name: 'More filter controls' }).getByRole('menuitem', { name: 'Clear all' }).click();
    await expect(page.getByRole('toolbar', { name: 'Levels' }).getByRole('button', { name: /^ERR/ })).toHaveAttribute('aria-pressed', 'false');
  });

  test('the message column stays readable at 900 px and the grid does not overflow the window', async ({ page }) => {
    await resize(page, 900);
    const msg = page.locator('imt-events-grid [role=row].row').first().locator('.msg');
    await expect(msg).toBeVisible();
    expect((await msg.boundingBox())!.width).toBeGreaterThan(200);
    const group = (await page.locator('.dv-groupview').filter({ has: page.getByRole('tab', { name: 'Events', exact: true }) }).boundingBox())!;
    expect(group.x + group.width).toBeLessThanOrEqual(900 + 1);
  });

  test('600 px: still usable, no sideways scroll, accessible', async ({ page }, testInfo) => {
    await resize(page, 600);
    await expect(sidebar(page)).toHaveCount(0);
    await expect(more(page)).toBeVisible();
    await expect(page.getByTestId('query-field')).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    expect(await noHorizontalScroll(page)).toBe(true);
    await expect(page.locator('imt-status-bar .low').first()).toBeHidden();
    await expectNoSeriousViolations(page, testInfo, 'narrow-600');
  });

  // Known gap: the one-row filter bar (search 200 px + level chips 447 px + buttons) is wider than the window below
  // ~1100 px, so its right-hand buttons (View options, Add filter) are clipped; it folds only below 860 px and does
  // not wrap. Enable once the bar wraps or folds earlier.
  for (const width of [1024, 900, 600]) {
    test(`the filter bar's buttons stay inside a ${width} px window`, async ({ page }) => {
      await resize(page, width);
      const right = await page.getByRole('button', { name: 'View options' }).evaluate((el) => el.getBoundingClientRect().right);
      expect(right).toBeLessThanOrEqual(width);
    });
  }

  test('widening the window brings the Explorer back; Ctrl+B opens it at any width', async ({ page }) => {
    await resize(page, 900);
    await expect(sidebar(page)).toHaveCount(0);
    await resize(page, 1440);
    await expect(sidebar(page)).toBeVisible();
    await expect(more(page)).toHaveCount(0);
    await resize(page, 900);
    await expect(sidebar(page)).toHaveCount(0);
    await page.keyboard.press('ControlOrMeta+b');
    await expect(sidebar(page)).toBeVisible();
  });

  test('an Explorer the user closed stays closed when the window widens', async ({ page }) => {
    await page.keyboard.press('ControlOrMeta+b');
    await expect(sidebar(page)).toHaveCount(0);
    await resize(page, 900);
    await resize(page, 1440);
    await expect(sidebar(page)).toHaveCount(0);
  });
});
