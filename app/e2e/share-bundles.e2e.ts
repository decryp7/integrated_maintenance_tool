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

test('shares the workspace as a project file and opens an old bundle read-only', async ({ page }, testInfo) => {
  await openApp(page);
  // Sharing saves the project: the .imtz is self-contained and opens anywhere (no bundle dialog any more).
  await runCommand(page, 'Share as project file');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: 'Saved Untitled' })).toBeVisible();

  await runCommand(page, 'Open bundle');
  const banner = page.getByTestId('bundle-banner');
  await expect(banner).toContainText('(read-only)');
  await expectNoSeriousViolations(page, testInfo, 'bundle-banner');
  await banner.getByRole('button', { name: 'Close' }).click();
  await expect(banner).toHaveCount(0);
});
