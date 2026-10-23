import { expect, Locator, Page } from '@playwright/test';
import { openAppWith } from './mock-url';

/** Opens the app with the scripted AI mock (fast streaming) plus `extra` switches, and waits for the grid. */
export async function openAiApp(page: Page, extra = ''): Promise<void> {
  await openAppWith(page, `?mockAiStepMs=5${extra ? `&${extra}` : ''}`);
  await expect(page.locator('debrief-events-grid [role=row].row').first()).toBeVisible();
}

/** Opens the assistant chat from the palette and returns the panel. */
export async function openChat(page: Page): Promise<Locator> {
  await runCommand(page, 'AI: Open the assistant chat');
  const panel = page.locator('debrief-ai-chat-panel');
  await expect(panel.getByRole('textbox', { name: 'Ask about the logs' })).toBeEnabled();
  return panel;
}

/** Sends a question and waits until the answer finished streaming (Send is back). */
export async function ask(panel: Locator, question: string): Promise<void> {
  await panel.getByRole('textbox', { name: 'Ask about the logs' }).fill(question);
  await panel.getByRole('button', { name: 'Send', exact: true }).click();
}

export async function waitAnswered(panel: Locator): Promise<void> {
  await expect(panel.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
  await expect(panel.locator('.thinking')).toHaveCount(0);
}

/** Right-clicks the first grid row and returns the context menu. */
export async function rowMenu(page: Page): Promise<Locator> {
  await page.locator('debrief-events-grid [role=row].row').first().click({ button: 'right' });
  const menu = page.getByRole('menu', { name: 'Actions' });
  await expect(menu).toBeVisible();
  return menu;
}

/** Runs a palette command by title (options are looked up in the palette only: the chat has <option>s too). */
export async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await expect(input).toBeFocused();
  await input.fill(title);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

/** Waits for the toast of a background AI answer ("Explanation ready") and opens it with View. */
export async function viewAnswer(page: Page, title: string): Promise<void> {
  const toast = page.getByTestId('notification').filter({ hasText: title });
  await expect(toast).toBeVisible({ timeout: 15_000 });
  await toast.getByTestId('notification-action-primary').click();
}

/** The dialog that shows an AI answer (opened by View). */
export function narrativeDialog(page: Page): Locator {
  return page.getByRole('dialog').filter({ has: page.getByTestId('ai-narrative') });
}
