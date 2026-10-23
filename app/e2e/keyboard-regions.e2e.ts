import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

/** The region (shell landmark or dock group) holding the focused element. */
function focusedRegion(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const el = document.activeElement;
    const region = el?.closest('[data-debrief-region]');
    if (region) return region.getAttribute('data-debrief-region');
    const group = el?.closest('.dv-groupview');
    if (!group) return null;
    return `group:${[...document.querySelectorAll('.dv-groupview')].indexOf(group) + 1}`;
  });
}

// G1: every region is at most three keys away, with the keyboard only.
test.describe('keyboard regions', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('F6 and Shift+F6 cycle the activity bar, side bar, filter bar, dock groups and status bar', async ({ page }, testInfo) => {
    await expect(page.getByRole('navigation', { name: 'Activity bar' })).toBeVisible();
    await expect(page.getByRole('complementary', { name: 'Side bar' })).toBeVisible();
    await expect(page.getByRole('search', { name: 'Filters' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Status bar' })).toBeVisible();

    await page.keyboard.press('F6');
    expect(await focusedRegion(page)).toBe('activity');
    await expectNoSeriousViolations(page, testInfo, 'region-focus', '[data-debrief-region="activity"]');
    await page.keyboard.press('F6');
    expect(await focusedRegion(page)).toBe('sidebar');
    await page.keyboard.press('F6');
    expect(await focusedRegion(page)).toBe('filters');
    await page.keyboard.press('F6');
    expect(await focusedRegion(page)).toBe('group:1');
    // Back to the start, then the status bar is one Shift+F6 away.
    await page.keyboard.press('Shift+F6');
    expect(await focusedRegion(page)).toBe('filters');
    await page.locator('body').evaluate((b) => (document.activeElement as HTMLElement | null)?.blur() ?? b);
    await page.keyboard.press('Shift+F6');
    expect(await focusedRegion(page)).toBe('status');
    await page.keyboard.press('F6');
    expect(await focusedRegion(page)).toBe('activity');
  });

  test('Ctrl+Alt+1… focus a dock group, Ctrl+0 the side bar, Ctrl+Shift+G the events grid', async ({ page }) => {
    const groups = await page.locator('.dv-groupview').count();
    expect(groups).toBeGreaterThan(1);
    for (let n = 1; n <= Math.min(groups, 3); n++) {
      await page.keyboard.press(`Control+Alt+Digit${n}`);
      expect(await focusedRegion(page)).toBe(`group:${n}`);
    }
    await page.keyboard.press('ControlOrMeta+0');
    expect(await focusedRegion(page)).toBe('sidebar');
    await page.keyboard.press('ControlOrMeta+Shift+G');
    await expect(page.getByRole('grid', { name: 'Events' })).toBeFocused();
  });

  test('the level chips are one tab stop; the arrows move between them', async ({ page }) => {
    const levels = page.getByRole('toolbar', { name: 'Levels' });
    const chips = levels.getByRole('button');
    // The chips re-render while the first counts arrive: focus the first chip once the bar has settled.
    await expect(chips).toHaveCount(5); // levels with no events (Verbose here) have no chip
    await expect(levels.getByRole('button', { name: /^ERR \d/ })).toBeVisible();
    await chips.first().focus();
    await expect(chips.first()).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(chips.nth(1)).toBeFocused();
    // The roving tab stop follows after the next render; Tab straight away would still see the old one.
    await expect(chips.nth(1)).toHaveAttribute('tabindex', '0');
    // Tab leaves the toolbar at once instead of visiting the other four chips.
    await page.keyboard.press('Tab');
    expect(await page.evaluate(() => !!document.activeElement?.closest('[aria-label="Levels"]'))).toBe(false);
    await page.keyboard.press('Shift+Tab');
    await expect(chips.nth(1)).toBeFocused();
  });
});
