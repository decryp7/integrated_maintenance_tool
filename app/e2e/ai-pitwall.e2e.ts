import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// Pitwall as the AI engine (ADR 0059) against the scripted AI mock (?mockPitwall=...).

async function runCommand(page: Page, text: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(text);
  await expect(page.locator('debrief-command-palette').getByRole('option').first()).toContainText(text);
  await page.keyboard.press('Enter');
}

const wizard = (page: Page) => page.getByRole('dialog', { name: 'Set up the AI assistant' });

async function openAiSettings(page: Page) {
  await page.keyboard.press('ControlOrMeta+Comma');
  const settings = page.getByRole('dialog', { name: 'Settings' });
  await settings.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  return settings;
}

test('the set-up offers Pitwall, shows the code and closes once Pitwall approved; Settings then says Pitwall', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockPitwall=running&mockAi=setup&mockAiStepMs=40');
  await runCommand(page, 'Set up AI assistant');
  const dialog = wizard(page);
  await expect(dialog.getByTestId('ai-setup-pitwall')).toContainText('Pitwall is running on this computer');
  await expect(dialog.getByTestId('ai-local-note')).toHaveCount(0);
  await expectNoSeriousViolations(page, testInfo, 'ai-setup-pitwall', '[role=dialog]');
  await dialog.getByTestId('ai-setup-use-pitwall').click();
  await expect(dialog.getByTestId('ai-setup-pitwall-code')).toHaveText('482 913');
  await expect(dialog.getByTestId('ai-setup-pitwall-cancel')).toBeVisible();
  await expect(dialog).toHaveCount(0);
  const settings = await openAiSettings(page);
  await expect(settings.getByTestId('ai-engine-status')).toContainText('Engine: Pitwall 1.0.4 · model qwen3-8b-q4km');
  await expect(settings.getByTestId('ai-pitwall-forget')).toBeVisible();
});

test('"Download a model for Debrief instead" shows the model list', async ({ page }) => {
  await openAppWith(page, '?mockPitwall=running&mockAi=setup&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  const dialog = wizard(page);
  await dialog.getByTestId('ai-setup-download-instead').click();
  await expect(dialog.getByTestId('ai-hardware')).toContainText('8 GB video memory');
  await expect(dialog.getByTestId('ai-model-qwen3.5-9b-q4km')).toContainText('Best');
  await expect(dialog.getByTestId('ai-local-note')).toContainText('Hugging Face');
});

test('a denied pairing shows the problem and can be tried again', async ({ page }) => {
  await openAppWith(page, '?mockPitwall=deny&mockAi=setup&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  const dialog = wizard(page);
  await dialog.getByTestId('ai-setup-use-pitwall').click();
  await expect(dialog.getByTestId('ai-setup-pitwall-error')).toHaveText('Pitwall denied the request.');
  await expect(dialog.getByTestId('ai-setup-use-pitwall')).toBeEnabled();
  await dialog.getByTestId('ai-setup-use-pitwall').click();
  await expect(dialog.getByTestId('ai-setup-pitwall-error')).toHaveText('Pitwall denied the request.');
});

test('Settings → AI offers to connect to a running Pitwall', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockPitwall=running&mockAiStepMs=5');
  const settings = await openAiSettings(page);
  await expect(settings.getByTestId('ai-engine-status')).toContainText('Pitwall is running on this computer');
  await expect(settings.getByTestId('ai-pitwall-connect')).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'ai-settings-pitwall', '[role=dialog]');
  await settings.getByTestId('ai-pitwall-connect').click();
  await expect(settings.getByTestId('ai-engine-status')).toContainText('Engine: Pitwall 1.0.4', { timeout: 10_000 });
});
