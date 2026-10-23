import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
}

// Dialogs are OS windows in the desktop app (ADR 0050); the browser build shows the same dialogs in the page, modal ones
// with a blocking backdrop and modeless ones as floating panels, so this suite exercises the difference.
test.describe('Dialog modality', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('a modeless dialog leaves the page usable and opens only once', async ({ page }) => {
    await runCommand(page, 'About Debrief');
    const dialog = page.getByRole('dialog', { name: 'About Debrief' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute('data-modality', 'modeless');
    await expect(dialog).toHaveAttribute('aria-modal', 'false');
    await expect(page.getByTestId('dialog-backdrop')).toHaveCount(0);

    // The page behind it still takes commands.
    await page.keyboard.press('ControlOrMeta+k');
    await expect(page.getByRole('combobox', { name: 'Type a command' })).toBeVisible();
    await page.keyboard.press('Escape');

    // Opening it again does not stack a second one.
    await runCommand(page, 'About Debrief');
    await expect(page.getByRole('dialog', { name: 'About Debrief' })).toHaveCount(1);
  });

  test('a modal dialog blocks the page behind a backdrop until it is answered', async ({ page }) => {
    await runCommand(page, 'Paste as log');
    const dialog = page.getByRole('dialog', { name: 'Paste as log' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute('data-modality', 'modal');
    await expect(dialog).toHaveAttribute('aria-modal', 'true');
    await expect(page.getByTestId('dialog-backdrop')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId('dialog-backdrop')).toHaveCount(0);
  });
});
