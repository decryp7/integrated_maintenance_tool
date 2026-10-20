import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await page.getByRole('option', { name: new RegExp(title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click();
  // The palette is gone before the next shortcut (Mod+K on an open palette would close it instead).
  await expect(input).toHaveCount(0);
}

test('shares the workspace as a bundle and opens a bundle read-only', async ({ page }, testInfo) => {
  await openApp(page);
  await runCommand(page, 'Share as bundle');
  const dialog = page.getByRole('dialog', { name: 'Share as bundle' });
  await expect(dialog.getByRole('status')).toContainText('before compression');
  await expectNoSeriousViolations(page, testInfo, 'bundle-dialog');
  await dialog.getByRole('textbox', { name: 'Bundle name' }).fill('Case 42');
  await dialog.getByLabel('Only the events matching the current filter (CLEF)').check();
  await dialog.getByLabel("Redact with the profile's rules").check();
  await expect(dialog.getByRole('status')).toContainText('files');
  await dialog.getByRole('button', { name: 'Save bundle…' }).click();
  await expect(dialog).toHaveCount(0);
  // A job writes the bundle; its message names the file.
  const done = page.getByTestId('notice-toast').filter({ hasText: 'Finished: Share bundle Case 42.imtz' });
  await expect(done).toContainText('Bundle written, redacted', { timeout: 15_000 });
  const written = await page.evaluate(() => (globalThis as unknown as { __imtMockShareWritten: { path: string; kind: string }[] }).__imtMockShareWritten);
  expect(written.some((w) => w.kind === 'imtz' && w.path.endsWith('.imtz'))).toBe(true);

  await runCommand(page, 'Open bundle');
  const banner = page.getByTestId('bundle-banner');
  await expect(banner).toContainText('Opened from shared (read-only)');
  await expectNoSeriousViolations(page, testInfo, 'bundle-banner');
  await banner.getByRole('button', { name: 'Close' }).click();
  await expect(banner).toHaveCount(0);
});
