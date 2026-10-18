import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// The inference runtime (llama-server builds, ADR 0009) in the set-up wizard and the model manager,
// against the scripted AI mock (src/app/core/mock/mock-ai-runtimes.ts).

async function runCommand(page: Page, text: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(text);
  await expect(page.getByRole('option').first()).toContainText(text);
  await page.keyboard.press('Enter');
}

test('without a runtime the wizard downloads the suggested build first, then sets up a model', async ({
  page,
}, testInfo) => {
  await openAppWith(page, '?mockAi=setup&mockAiRuntime=none&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  const dialog = page.getByRole('dialog', { name: 'Set up the AI assistant' });
  const step = dialog.getByTestId('ai-setup-runtime');
  await expect(step).toContainText('Inference runtime');
  await expect(dialog.getByTestId('ai-runtime-suggested')).toContainText(
    'CUDA 13 (NVIDIA) is recommended',
  );
  await expect(dialog.getByTestId('ai-runtime-continue')).toBeDisabled();
  await expect(dialog.getByTestId('ai-runtime-cuda-13')).toContainText('Suggested');
  await expectNoSeriousViolations(page, testInfo, 'ai-setup-runtime', '[role=dialog]');

  await dialog.getByTestId('ai-runtime-download-suggested').click();
  await expect(dialog.getByTestId('ai-runtime-progress')).toBeVisible();
  await expect(dialog.getByTestId('ai-runtime-ready')).toContainText(
    'CUDA 13 (NVIDIA) is installed',
  );
  await expect(dialog.getByTestId('ai-runtime-cuda-13')).toContainText('Active');
  await dialog.getByTestId('ai-runtime-continue').click();

  await expect(dialog.getByTestId('ai-hardware')).toContainText('8 GB video memory');
  await dialog.getByTestId('ai-setup-start').click();
  await expect(dialog.getByTestId('ai-setup-ready')).toContainText('CUDA');
});

test('a runtime download can be cancelled', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiRuntime=none&mockAiStepMs=40');
  await runCommand(page, 'Set up AI assistant');
  const dialog = page.getByRole('dialog', { name: 'Set up the AI assistant' });
  await dialog.getByRole('button', { name: 'Download CPU' }).click();
  await expect(
    dialog.getByRole('progressbar', { name: 'Runtime download progress' }),
  ).toBeVisible();
  await dialog.getByTestId('ai-runtime-cancel').click();
  await expect(dialog.getByTestId('ai-runtime-cancelled')).toBeVisible();
  await expect(dialog.getByTestId('ai-runtime-continue')).toBeDisabled();
});

test('a failed runtime download says why and offers to try again', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiRuntime=none&mockAiFail=runtime&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  const dialog = page.getByRole('dialog', { name: 'Set up the AI assistant' });
  await dialog.getByTestId('ai-runtime-download-suggested').click();
  const error = dialog.getByTestId('ai-runtime-error');
  await expect(error).toContainText('connection to github.com was reset');
  await expect(error.getByRole('button', { name: 'Try again' })).toBeVisible();
});

test('the model manager lists the runtimes, switches the build and deletes after confirmation', async ({
  page,
}, testInfo) => {
  await openAppWith(page, '?mockAiStepMs=5');
  await runCommand(page, 'AI: Manage models');
  const dialog = page.getByRole('dialog', { name: 'AI models' });
  const vulkan = dialog.getByTestId('ai-runtime-vulkan');
  await expect(vulkan).toContainText('Active');
  await expect(vulkan).toContainText('RTX 4060');
  await expect(dialog.getByTestId('ai-runtime-cuda-13')).toContainText('Suggested');
  await expect(dialog.getByTestId('ai-runtime-outdated-b10012-cpu')).toContainText(
    'no longer offered',
  );
  await expectNoSeriousViolations(page, testInfo, 'ai-runtimes', '[role=dialog]');

  await dialog.getByRole('button', { name: 'Download CUDA 13 (NVIDIA)' }).click();
  await expect(dialog.getByTestId('ai-runtime-ready')).toBeVisible();
  // The automatic choice now prefers CUDA; fix Vulkan, then go back to automatic.
  await expect(dialog.getByTestId('ai-runtime-cuda-13')).toContainText('Active');
  await dialog.getByRole('button', { name: 'Use this: Vulkan (any GPU vendor)' }).click();
  await expect(dialog.getByTestId('ai-runtime-choice')).toContainText('Fixed to Vulkan');
  await expect(vulkan).toContainText('Active');
  await dialog.getByTestId('ai-runtime-auto').click();
  await expect(dialog.getByTestId('ai-runtime-choice')).toContainText('Automatic choice');

  await dialog.getByRole('button', { name: 'Delete CPU b10012' }).click();
  const confirm = page.getByRole('dialog', { name: 'Delete inference runtime?' });
  await expect(confirm.getByTestId('confirm-message')).toContainText('release b10012');
  await confirm.getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.getByTestId('ai-runtime-outdated-b10012-cpu')).toHaveCount(0);
});

test('a build that finds no GPU is flagged and the download is offered first', async ({ page }) => {
  await openAppWith(page, '?mockAiRuntime=broken&mockAiStepMs=5');
  await runCommand(page, 'AI: Manage models');
  const dialog = page.getByRole('dialog', { name: 'AI models' });
  await expect(dialog.getByTestId('ai-runtime-vulkan')).toContainText('does not work here');
  await expect(dialog.getByRole('button', { name: 'Use this: Vulkan (any GPU vendor)' })).toBeDisabled();
  await expect(dialog.getByTestId('ai-runtime-suggested')).toContainText('CUDA 13 (NVIDIA)');
});
