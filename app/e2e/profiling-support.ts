import { expect, Page } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';
import { chooseMenu } from './menu-support';

/**
 * Opens the app (browser build, mock backends) and opens the demo captures through File › Open
 * profile…: the browser "dialog" returns orders, orders-regression and three heap snapshots.
 * `search` passes mock switches such as `?mockProfilingFail=hotspots:1`.
 */
export async function openProfiles(page: Page, search = ''): Promise<void> {
  await page.goto(`/${search}`);
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await expect(page.locator('imt-status-bar')).toContainText('events');
  await chooseMenu(page, 'File', 'Open profile…');
  await expect(hotspots(page).locator('tbody tr[aria-rowindex]').first()).toBeVisible();
}

/** Runs a palette command by (part of) its title. */
export async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

export const hotspots = (page: Page) => page.locator('imt-hotspots-view');
export const callTree = (page: Page) => page.locator('imt-call-tree-view');
export const flame = (page: Page) => page.locator('imt-flame-view');
export const compare = (page: Page) => page.locator('imt-compare-view');
export const threads = (page: Page) => page.locator('imt-thread-timeline-view');
export const findings = (page: Page) => page.locator('imt-findings-view');
export const heap = (page: Page) => page.locator('imt-heap-view');

/** Names in the hotspots table, top to bottom (rows in the DOM). */
export async function hotspotNames(page: Page): Promise<string[]> {
  return hotspots(page).locator('tbody td.name button').allInnerTexts();
}
