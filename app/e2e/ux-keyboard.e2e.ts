import { expect, test } from '@playwright/test';
import { openApp } from './app';

// Usability review 2026-10-23 (U9): 72 Tab presses from the window start to the events grid. Toolbars are roving
// groups now (one Tab stop, arrows inside); F6 still cycles the regions.
test('few Tab presses reach the events grid and the activity bar is one stop', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  await page.locator('body').click({ position: { x: 1, y: 1 } });
  let presses = 0;
  const inActivity: string[] = [];
  for (; presses < 80; ) {
    await page.keyboard.press('Tab');
    presses++;
    const where = await page.evaluate(() => {
      const a = document.activeElement as HTMLElement;
      return { grid: !!a.closest('debrief-events-grid'), activity: a.closest('debrief-activity-bar') ? a.getAttribute('data-activity-id') : null };
    });
    if (where.activity) inActivity.push(where.activity);
    if (where.grid) break;
  }
  expect(inActivity).toHaveLength(1);
  expect(presses).toBeLessThanOrEqual(45);
});

test('arrows move inside the activity bar and the bar keeps one tab stop', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  const buttons = page.locator('debrief-activity-bar button.act');
  expect(await buttons.count()).toBeGreaterThan(5);
  await expect(page.locator('debrief-activity-bar button.act[tabindex="0"]')).toHaveCount(1);
  await buttons.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(buttons.nth(1)).toBeFocused();
  await expect(buttons.nth(1)).toHaveAttribute('tabindex', '0');
  await expect(page.locator('debrief-activity-bar button.act[tabindex="0"]')).toHaveCount(1);
  await page.keyboard.press('End');
  await expect(buttons.last()).toBeFocused();
});
