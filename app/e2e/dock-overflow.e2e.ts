import { expect, Page, test } from '@playwright/test';
import { openApp, openView } from './app';

// A tab strip too narrow for its tabs ends in a chip ("⌄ 2") that lists the hidden tabs.

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const chip = (page: Page) => page.getByRole('button', { name: '2 more tabs' });
const list = (page: Page) => page.locator('.dv-tabs-overflow-container');

/** The dock narrowed so the Timeline group (Timeline, Registry, Metrics) shows one tab and hides two. */
async function narrowDock(page: Page): Promise<void> {
  await openApp(page);
  await openView(page, 'metrics');
  await page.locator('.debrief-dock-mount').evaluate((el: HTMLElement) => (el.style.flex = '0 0 260px'));
  await expect(chip(page)).toBeVisible();
}

/** The element on top at the middle of a locator's box. */
async function topmostIsInside(page: Page, selector: string): Promise<boolean> {
  const b = (await page.locator(selector).first().boundingBox())!;
  return page.evaluate(([x, y, sel]) => !!document.elementFromPoint(x as number, y as number)?.closest(sel as string), [b.x + b.width / 2, b.y + b.height / 2, selector]);
}

test.describe('tab overflow list', () => {
  test('the chip is a named button with a tooltip; clicking it lists the hidden tabs on top, and a row opens its tab', async ({ page }) => {
    await narrowDock(page);
    await expect(chip(page)).toHaveAttribute('title', '2 more tabs');
    await chip(page).click();
    await expect(list(page)).toBeVisible();
    expect(await topmostIsInside(page, '.dv-tabs-overflow-container')).toBe(true);
    await list(page).locator('.dv-tab', { hasText: 'Registry' }).click();
    await expect(list(page)).toHaveCount(0);
    await expect(tab(page, 'Registry')).toHaveAttribute('aria-selected', 'true');
  });

  test('the chip opens with the keyboard', async ({ page }) => {
    await narrowDock(page);
    await chip(page).focus();
    await page.keyboard.press('Enter');
    await expect(list(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(list(page)).toHaveCount(0);
    await chip(page).focus();
    await page.keyboard.press(' ');
    await expect(list(page)).toBeVisible();
  });
});
