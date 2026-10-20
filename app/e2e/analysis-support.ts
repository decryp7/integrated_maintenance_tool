import { expect, Page } from '@playwright/test';

/** Runs a command from the palette by its title. */
export async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await expect(input).toBeFocused();
  await input.fill(title);
  await expect(page.getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

/** Opens the query bar (Ctrl/⌘+/: the Focused layout does not open it by itself) and returns its input. */
export async function openQuery(page: Page) {
  const input = page.getByRole('combobox', { name: 'Query' });
  if (!(await input.isVisible())) await page.keyboard.press('ControlOrMeta+/');
  await expect(input).toBeVisible();
  return input;
}
