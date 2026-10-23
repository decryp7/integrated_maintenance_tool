import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// The AI setup wizard and Settings → AI against the scripted AI mock (src/app/core/mock/mock-ai-backend.ts).

async function runCommand(page: Page, text: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(text);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(text);
  await page.keyboard.press('Enter');
}

const wizard = (page: Page) => page.getByRole('dialog', { name: 'Set up the AI assistant' });

test('the set-up is a quick modal choice: Download and set up closes it and the rest runs in the background', async ({ page }, testInfo) => {
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
  await expect(dialog).toHaveAttribute('data-modality', 'modal');
  await expectNoSeriousViolations(page, testInfo, 'ai-setup-plan', '[role=dialog]');

  await dialog.getByTestId('ai-setup-start').click();
  // The window is gone at once; there is no progress, step or cancel in it.
  await expect(dialog).toHaveCount(0);
  // The assistant pane says how far it is, and a message announces the end.
  await expect(page.getByTestId('notice-toast').filter({ hasText: 'AI assistant ready' })).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('notice-toast').filter({ hasText: 'AI assistant ready' }).getByRole('button', { name: 'Open the assistant' })).toBeVisible();
});

test('the assistant pane shows "Setting up — 1.2 of 5.6 GB" while the background set-up runs', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiStepMs=60');
  await runCommand(page, 'Set up AI assistant');
  await wizard(page).getByTestId('ai-setup-start').click();
  await runCommand(page, 'AI: Open the assistant chat');
  await expect(page.getByTestId('ai-setting-up')).toContainText(/Setting up — .* of 5\.6 GB/, { timeout: 20_000 });
});

test('Settings → AI offers set-up', async ({ page }) => {
  await openAppWith(page, '');
  await page.keyboard.press('ControlOrMeta+Comma');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  await expect(settings.getByTestId('ai-settings-status')).toContainText('Vulkan');
  await settings.getByTestId('ai-settings-setup').click();
  await expect(wizard(page)).toBeVisible();
});

test('AI is hidden when it is unavailable', async ({ page }) => {
  await openAppWith(page, '?mockAi=off');
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill('Set up AI assistant');
  await expect(page.locator('debrief-command-palette').getByRole('option', { name: /Set up AI assistant/ })).toHaveCount(0);
  await input.fill('AI:');
  await expect(page.locator('debrief-command-palette').getByRole('option', { name: /^AI:/ })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await page.keyboard.press('ControlOrMeta+Comma');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  await expect(settings.getByTestId('ai-settings-unavailable')).toContainText('not available');
  await expect(settings.getByTestId('ai-settings-setup')).toHaveCount(0);
});
