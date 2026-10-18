import { expect, Page } from '@playwright/test';

/** How long the first load of the demo workspace may take (it is slower while the whole suite runs in parallel). */
export const APP_LOAD_TIMEOUT = 15_000;

/**
 * How long an error snapshot may take to appear: html-to-image clones and styles the whole window, which takes
 * about 2 s alone and several seconds while the suite runs in parallel, more than the default 5 s expect timeout.
 */
export const CAPTURE_TIMEOUT = 15_000;

/** Opens the app in the browser build (MockBackend) and waits for the demo workspace to load. */
export async function openApp(page: Page): Promise<void> {
  await page.goto('/');
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await expect(page.locator('imt-status-bar')).toContainText('events');
  await expect(page.locator('imt-timeline-panel .lane').first()).toBeVisible();
  await expect(page.locator('imt-events-grid [role=row].row').first()).toBeVisible();
}

/** Parses a count like "40,123" or "40.1k" shown in the UI into a number. */
export function parseCount(text: string): number {
  return Number(text.replace(/[^\d]/g, ''));
}

/** Reads the unfiltered total once the demo workspace is loaded (the demo data may change). */
export async function initialTotal(page: Page): Promise<string> {
  const total = gridTotal(page);
  await expect(total).toHaveText(/^\d[\d,]*$/);
  return total.innerText();
}

/** Total of matching events as shown in the grid bar. */
export function gridTotal(page: Page) {
  return page.locator('imt-events-grid .bar .total');
}

/**
 * Resolves once the app has rendered what is already scheduled: after a frame and then a task (Angular renders on
 * whichever comes first). The sync point for asserting that something did NOT happen, instead of a fixed sleep.
 */
export async function nextRender(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0))));
}

/**
 * Docks the inspector (by default it is an auto-hidden tool window on the right edge that only slides out while an
 * event is selected): slides it out from its side-bar tab and pins it. Does nothing when it is docked already.
 */
export async function ensureInspector(page: Page): Promise<void> {
  const tab = page.locator('.imt-ah-tab[data-ah-id="inspector"]');
  if ((await tab.count()) === 0) {
    await expect(page.locator('imt-inspector')).toBeAttached();
    return;
  }
  const pin = page.locator('#imt-ah-flyout').getByRole('button', { name: 'Pin Inspector' });
  if (!(await pin.isVisible())) await tab.click();
  await pin.click();
  await expect(tab).toHaveCount(0);
  await expect(page.locator('imt-inspector')).toBeVisible();
}

/** Views that are not in the default layout, by id, with the activity-bar button that opens them. */
const VIEW_BUTTONS: Readonly<Record<string, string>> = { metrics: 'Metrics' };

/** Opens a view that is not part of the minimal default layout (e.g. Metrics) and waits for its tab. */
export async function openView(page: Page, id: keyof typeof VIEW_BUTTONS | string): Promise<void> {
  const name = VIEW_BUTTONS[id] ?? id;
  const tab = page.locator('.dv-tab', { has: page.locator(`.imt-tab[data-panel-id="${id}"]`) });
  if ((await tab.count()) === 0) await page.getByRole('button', { name, exact: true }).click();
  await expect(tab).toBeVisible();
}

/**
 * Slides the auto-hidden inspector back in after a selection slid it out, so it does not cover the panels under it.
 * Does nothing when nothing is slid out.
 */
export async function hideInspectorFlyout(page: Page): Promise<void> {
  await nextRender(page);
  const flyout = page.locator('#imt-ah-flyout');
  if (!(await flyout.isVisible())) return;
  await flyout.getByRole('button', { name: /^Hide / }).click();
  await expect(flyout).toHaveCount(0);
}
