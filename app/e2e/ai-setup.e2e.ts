import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// The AI setup wizard and Settings → AI against the scripted AI mock (src/app/core/mock/mock-ai-backend.ts).

async function runCommand(page: Page, text: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(text);
  await expect(page.getByRole('option').first()).toContainText(text);
  await page.keyboard.press('Enter');
}

const wizard = (page: Page) => page.getByRole('dialog', { name: 'Set up the AI assistant' });

test('the setup wizard shows the hardware and models, downloads and reaches Ready', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockAi=setup&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  const dialog = wizard(page);
  await expect(dialog.getByTestId('ai-hardware')).toContainText('8 GB video memory');
  await expect(dialog.getByTestId('ai-local-note')).toContainText('fully on this computer');
  await expect(dialog.getByTestId('ai-local-note')).toContainText('Hugging Face');
  const best = dialog.getByTestId('ai-model-qwen3.5-9b-q4km');
  await expect(best).toContainText('Best');
  await expect(best).toContainText('Fits comfortably');
  await expect(best).toContainText('Tool calling');
  await expect(dialog.getByTestId('ai-model-qwen3.5-4b-q4km')).toContainText('Fastest');
  await expect(dialog.getByTestId('ai-model-qwen3.5-32b-q4km')).toContainText('Too large');
  await expectNoSeriousViolations(page, testInfo, 'ai-setup-plan', '[role=dialog]');

  await dialog.getByTestId('ai-setup-start').click();
  await expect(dialog.getByRole('progressbar', { name: 'Download progress' })).toBeVisible();
  await expect(dialog.getByTestId('ai-setup-bytes')).toContainText('of 5.6 GB');
  await expect(dialog.getByTestId('ai-setup-ready')).toContainText('Vulkan');
  await expectNoSeriousViolations(page, testInfo, 'ai-setup-ready', '[role=dialog]');
  await dialog.getByRole('button', { name: 'Close' }).first().click();
  await expect(dialog).toHaveCount(0);
});

test('a running set-up can be cancelled', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiStepMs=60');
  await runCommand(page, 'Set up AI assistant');
  const dialog = wizard(page);
  await dialog.getByTestId('ai-setup-start').click();
  await expect(dialog.getByTestId('ai-setup-stage')).toBeVisible();
  await dialog.getByTestId('ai-setup-cancel').click();
  await expect(dialog.getByTestId('ai-setup-cancelled')).toBeVisible();
});

test('Settings → AI offers set-up and a write-only Brave key', async ({ page }) => {
  await openAppWith(page, '');
  await page.keyboard.press('ControlOrMeta+Comma');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  await expect(settings.getByTestId('ai-settings-status')).toContainText('Vulkan');
  const key = settings.getByLabel('Brave Search API key');
  await expect(key).toHaveAttribute('type', 'password');
  await key.fill('BSA-123');
  await settings.getByRole('button', { name: 'Save key' }).click();
  await expect(settings.getByTestId('ai-key-state')).toContainText('A key is stored');
  await expect(key).toHaveValue('');
  await settings.getByTestId('ai-settings-setup').click();
  await expect(wizard(page)).toBeVisible();
});

test('AI is hidden when it is unavailable', async ({ page }) => {
  await openAppWith(page, '?mockAi=off');
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill('Set up AI assistant');
  await expect(page.getByRole('option', { name: /Set up AI assistant/ })).toHaveCount(0);
  await input.fill('AI:');
  await expect(page.getByRole('option', { name: /^AI:/ })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await page.keyboard.press('ControlOrMeta+Comma');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  await expect(settings.getByTestId('ai-settings-unavailable')).toContainText('not available');
  await expect(settings.getByTestId('ai-settings-setup')).toHaveCount(0);
});
