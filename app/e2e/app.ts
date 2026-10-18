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
