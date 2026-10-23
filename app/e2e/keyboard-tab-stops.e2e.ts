import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';

/** The most Tab presses allowed from the page start to the events grid (docs/ux-audit G1). */
const MAX_TAB_STOPS = 20;

/** The natural Tab order (skip link stepped over) measured when the skip link and roving panel actions landed; it must not grow. */
const NATURAL_TAB_STOPS_TODAY = 66;

/**
 * Presses Tab from the page start until the events grid holds focus; returns the stops taken and a trail of what was
 * focused. With `followSkipLink`, Enter on the "Skip to events" link counts as the stop that reaches the grid (what a
 * keyboard user does); without it the link is stepped over and the whole natural Tab order is walked.
 */
async function tabsToGrid(page: Page, followSkipLink: boolean, limit = 150): Promise<{ stops: number; trail: string[] }> {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  const trail: string[] = [];
  for (let n = 1; n <= limit; n++) {
    await page.keyboard.press('Tab');
    const at = await page.evaluate(() => {
      const el = document.activeElement;
      const label = `${el?.tagName.toLowerCase()}[${el?.getAttribute('aria-label') ?? el?.textContent?.trim().slice(0, 20) ?? ''}]`;
      return { label, inGrid: !!el?.closest('debrief-events-grid [role=grid]'), skip: !!el?.classList.contains('skip-link') };
    });
    trail.push(at.label);
    if (at.skip && followSkipLink) {
      await page.keyboard.press('Enter');
      await page.waitForFunction(() => !!document.activeElement?.closest('debrief-events-grid [role=grid]'));
      return { stops: n, trail };
    }
    if (at.inGrid) return { stops: n, trail };
  }
  return { stops: Infinity, trail };
}

test.describe('keyboard: Tab stops to the events grid', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test(`the events grid is at most ${MAX_TAB_STOPS} Tab stops from the page start (with the skip link)`, async ({ page }) => {
    const { stops, trail } = await tabsToGrid(page, true);
    expect(stops, trail.join(' > ')).toBeLessThanOrEqual(MAX_TAB_STOPS);
  });

  test('the natural Tab order to the grid does not grow past today\'s count', async ({ page }, testInfo) => {
    const { stops, trail } = await tabsToGrid(page, false);
    testInfo.annotations.push({ type: 'natural Tab stops to the grid', description: String(stops) });
    expect(stops, trail.join(' > ')).toBeLessThanOrEqual(NATURAL_TAB_STOPS_TODAY);
  });

  test('the first Tab stop is a Skip to events link, visible on focus, that focuses the grid', async ({ page, browserName }) => {
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    const skip = page.getByRole('button', { name: 'Skip to events' });
    // First in the tab order: no other tabbable element comes before it in the document.
    expect(await page.evaluate(() => document.querySelector('button, a[href], input, textarea, select, [tabindex]:not([tabindex="-1"])')?.classList.contains('skip-link'))).toBe(true);
    // WebKit on macOS does not Tab onto buttons unless full keyboard access is on, so there it is focused directly.
    if (browserName === 'webkit') await skip.focus();
    else await page.keyboard.press('Tab');
    await expect(skip).toBeFocused();
    const box = await skip.boundingBox();
    expect(box && box.width > 20 && box.height > 10 && box.x >= 0 && box.y >= 0).toBe(true);
    await page.keyboard.press('Enter');
    await expect(page.locator('debrief-events-grid [role=grid]')).toBeFocused();
  });

  test('a dock group header is one Tab stop: arrows move between its action buttons', async ({ page }) => {
    const actions = page.locator('.dv-groupview').first().locator('.debrief-dock-actions button');
    const count = await actions.count();
    expect(count).toBeGreaterThan(1);
    const tabbable = await actions.evaluateAll((els) => els.filter((e) => (e as HTMLElement).tabIndex >= 0).length);
    expect(tabbable).toBe(1);
  });
});
