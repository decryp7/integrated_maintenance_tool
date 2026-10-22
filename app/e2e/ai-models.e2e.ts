import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// The AI model manager against the scripted AI mock (one installed, active model by default).

async function openModels(page: Page) {
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('AI: Manage models');
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText('AI: Manage models');
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'AI models' });
  await expect(dialog).toBeVisible();
  return dialog;
}

test('the model manager lists models and deletes only after confirmation', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockAiStepMs=5');
  const dialog = await openModels(page);
  const row = dialog.getByTestId('ai-installed-Qwen3.5-4B-Q4_K_M.gguf');
  await expect(row).toContainText('Active');
  await expect(row).toContainText('Fits comfortably');
  await expect(dialog.getByTestId('ai-engine-status')).toContainText('Vulkan');
  await expectNoSeriousViolations(page, testInfo, 'ai-models', '[role=dialog]');

  await row.getByRole('button', { name: 'Delete Qwen3.5 4B' }).click();
  const confirm = page.getByRole('dialog', { name: 'Delete model?' });
  await expect(confirm.getByTestId('confirm-message')).toContainText('no model until you choose another');
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await expect(row).toBeVisible();

  await row.getByRole('button', { name: 'Delete Qwen3.5 4B' }).click();
  await page.getByRole('dialog', { name: 'Delete model?' }).getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.getByTestId('ai-no-models')).toBeVisible();
  await expect(dialog.getByTestId('ai-engine-status')).toContainText('No model set up');
});

test('a local GGUF can be added and made active', async ({ page }) => {
  await openAppWith(page, '?mockAiStepMs=5');
  const dialog = await openModels(page);
  await dialog.getByTestId('ai-add-local').click();
  const row = dialog.getByTestId('ai-installed-shared.gguf');
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: 'Use shared' }).click();
  await expect(dialog.getByTestId('ai-model-progress')).toContainText('is now active');
  await expect(row).toContainText('Active');
});
