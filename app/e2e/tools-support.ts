import { expect, Page } from '@playwright/test';

/**
 * Opens the app with the mock tools backend switched by query parameters:
 * mock-safe=1, mock-crash=1, mock-elevated=1, mock-os=mac, mock-denied=1, mock-latency=<ms>.
 */
export async function openAppWith(page: Page, search = ''): Promise<void> {
  await page.goto(`/${search}`);
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
  await expect(page.locator('imt-status-bar')).toContainText('events');
  await expect(page.locator('imt-events-grid [role=row].row').first()).toBeVisible();
  // The lifecycle extras are mounted into the title bar after start-up.
  await expect(page.locator('imt-title-extras')).toBeAttached();
}

/** Runs a command from the palette by (part of) its title. */
export async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

/** The number of sources shown in the status bar. */
export async function sourceCount(page: Page): Promise<number> {
  const text = await page.locator('imt-status-bar').innerText();
  return Number(/(\d+) sources?/.exec(text)?.[1] ?? 0);
}

/** Opens a panel from the View menu. */
export async function openView(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'View', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name }).click();
}
