import { expect, Page } from '@playwright/test';

/** Runs a command from the palette by its title. */
export async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await expect(input).toBeFocused();
  await input.fill(title);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

/** The filter bar's one search field (words search messages live; a query runs with Enter or Ctrl/Cmd+Enter). */
export function searchField(page: Page) {
  return page.getByTestId('query-field');
}

/** Returns the search field, visible and focused (it is always in the filter bar; Ctrl/Cmd+/ focuses it). */
export async function openQuery(page: Page) {
  const input = searchField(page);
  await expect(input).toBeVisible();
  await input.click();
  await expect(input).toBeFocused();
  return input;
}
