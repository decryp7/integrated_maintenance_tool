import { expect, Page } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';
import { showView } from './menu-support';

/**
 * Opens the app with the mock tools backend switched by query parameters:
 * mock-safe=1, mock-crash=1, mock-elevated=1, mock-os=mac, mock-denied=1, mock-latency=<ms>,
 * mock-tools=quarantined|inactive|disabled (state of the tools plugin).
 */
export async function openAppWith(page: Page, search = ''): Promise<void> {
  // These tests are about the tools, not Home (which opens by itself after an import and would take the document tab).
  await page.addInitScript(() => localStorage.setItem('debrief.home.autoOpen.v1', 'false'));
  await page.goto(`/${search}`);
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await expect(page.locator('debrief-status-bar')).toContainText('events');
  await expect(page.locator('debrief-events-grid [role=row].row').first()).toBeVisible();
  // The lifecycle extras are mounted into the title bar after start-up.
  await expect(page.locator('debrief-title-extras')).toBeAttached();
}

/** Runs a command from the palette by (part of) its title. */
export async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

/** The number of sources shown in the status bar. */
export async function sourceCount(page: Page): Promise<number> {
  const text = await page.locator('debrief-status-bar').innerText();
  return Number(/(\d+) sources?/.exec(text)?.[1] ?? 0);
}

/** Opens a panel (View ▸ Other windows…, which lists every view). */
export async function openView(page: Page, name: string): Promise<void> {
  await showView(page, name);
}
