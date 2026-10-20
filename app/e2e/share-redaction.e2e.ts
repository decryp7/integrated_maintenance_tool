import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await page.getByRole('option', { name: new RegExp(title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click();
  // The palette is gone before the test goes on (keys would otherwise go to it).
  await expect(input).toHaveCount(0);
}

test('the redacted view masks the grid until it is turned off', async ({ page }, testInfo) => {
  await openApp(page);
  await page.getByPlaceholder(/Search messages/).fill('listening');
  const row = page.locator('imt-events-grid [role=row].row').first();
  await expect(row).toContainText('0.0.0.0');

  await runCommand(page, 'Toggle redacted view');
  const banner = page.getByTestId('redacted-banner');
  await expect(banner).toBeVisible();
  await expect(row).toContainText('<ipv4>');
  await expect(row).not.toContainText('0.0.0.0');
  await expectNoSeriousViolations(page, testInfo, 'redacted-view');

  await banner.getByRole('button', { name: 'Turn off' }).click();
  await expect(banner).toHaveCount(0);
  await expect(row).toContainText('0.0.0.0');

  // The keyboard shortcut toggles it too, and is listed in the cheat sheet.
  await page.keyboard.press('ControlOrMeta+Shift+x');
  await expect(banner).toBeVisible();
  await page.keyboard.press('ControlOrMeta+Shift+x');
  await expect(banner).toHaveCount(0);
  await page.keyboard.press('F1');
  const sheet = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await sheet.getByRole('searchbox', { name: 'Search shortcuts' }).fill('redacted');
  await expect(sheet.locator('.cs-row')).toHaveCount(1);
  await expect(sheet.locator('.cs-row')).toContainText('Toggle redacted view');
});

test('redaction settings preview the profile rules live', async ({ page }, testInfo) => {
  await openApp(page);
  await runCommand(page, 'Redaction settings');
  const dialog = page.getByRole('dialog', { name: 'Redaction settings' });
  const preview = dialog.getByTestId('redaction-preview');
  await expect(preview).toContainText('<email>');
  await expectNoSeriousViolations(page, testInfo, 'redaction-settings');
  await dialog.getByRole('textbox', { name: 'Sample text' }).fill('from 10.1.2.3 as bob@example.org');
  await expect(preview).toHaveText('from <ipv4> as <email>');
  await dialog.getByLabel('Replace values by').selectOption('drop');
  await expect(preview).toHaveText('from  as');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
});

test('exports can be redacted with a preview of what is masked', async ({ page }) => {
  await openApp(page);
  await page.keyboard.press('ControlOrMeta+Shift+e');
  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await dialog.getByLabel("Redact with the profile's rules").check();
  await expect(dialog.locator('imt-redaction-summary')).toContainText('events change');
  await dialog.getByRole('button', { name: 'Export…' }).click();
  const done = page.getByTestId('notice-toast').filter({ hasText: 'Finished: Export events' });
  await expect(done).toContainText('(redacted,', { timeout: 15_000 });
});
