import { expect, Page } from '@playwright/test';

/**
 * Opens the app with the mock backend's URL switches (see src/app/core/mock/mock-options.ts), e.g.
 * `?mockTotal=50000000`. Only waits for the shell and the events grid, since some switches make
 * other panels fail on purpose.
 */
export async function openAppWith(page: Page, search: string, options: { grid?: boolean } = {}): Promise<void> {
  await page.goto(`/${search}`);
  await expect(page.locator('imt-status-bar')).toContainText('events');
  if (options.grid ?? true) await expect(page.locator('imt-events-grid')).toBeVisible();
}

/** The events grid's scroll container. */
export const viewport = (page: Page) => page.locator('imt-events-grid .viewport');

/** Rows currently in the DOM (never more than a screenful plus overscan, whatever the total). */
export const gridRows = (page: Page) => page.locator('imt-events-grid .row:not(.placeholder)');

/** Parses the id out of "Synthetic event 123". */
export function eventId(text: string | null): number {
  const m = /Synthetic event (\d+)/.exec(text ?? '');
  if (!m) throw new Error(`not a synthetic row: ${text}`);
  return Number(m[1]);
}
