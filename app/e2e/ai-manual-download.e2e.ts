import { expect, test } from '@playwright/test';
import { runCommand } from './ai-support';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// "Download manually" behind a TLS-inspecting proxy (Zscaler etc.), against the scripted AI mock:
// `?mockAiNet=tls` fails model and runtime downloads with the engine's certificate error.

const CUDA = ['/Downloads/llama-b11669-bin-win-cuda-13-x64.zip', '/Downloads/cudart-llama-bin-win-cuda-13-x64.zip'];

test('a runtime download blocked by a TLS-inspecting proxy is installed from the files downloaded by hand', async ({
  page,
}, testInfo) => {
  await openAppWith(page, `?mockAi=setup&mockAiRuntime=none&mockAiNet=tls&mockAiStepMs=5&mockOpen=${CUDA.join(',')}`);
  await runCommand(page, 'Set up AI assistant');
  const dialog = page.getByRole('dialog', { name: 'Set up the AI assistant' });
  await dialog.getByTestId('ai-runtime-download-suggested').click();
  await expect(dialog.getByTestId('ai-runtime-error')).toContainText('Zscaler');

  const manual = dialog.getByRole('region', { name: 'Download manually' });
  await expect(manual.getByTestId('ai-manual-kind')).toContainText('intercepts HTTPS');
  await expect(manual.getByTestId('ai-manual-file-0')).toContainText('llama-b11669-bin-win-cuda-13-x64.zip');
  await expect(manual.getByTestId('ai-manual-file-1')).toContainText('cudart-llama-bin-win-cuda-13-x64.zip');
  await expect(manual.getByTestId('ai-manual-sha').first()).toHaveText(/^[0-9a-f]{64}$/);
  await expect(manual.getByTestId('ai-manual-steps')).toContainText('Add downloaded files');
  await expect(manual.getByTestId('ai-manual-steps')).toContainText('github.com');
  await expect(manual.getByTestId('ai-manual-custom')).toContainText('Custom llama-server');
  await expect(manual.getByTestId('ai-manual-folder')).toContainText('runtimes');
  await expectNoSeriousViolations(page, testInfo, 'ai-manual-runtime', '[role=dialog]');

  await manual
    .getByRole('button', { name: 'Download manually: llama-b11669-bin-win-cuda-13-x64.zip' })
    .click();
  await expect(manual.getByTestId('ai-manual-status')).toContainText('opened in your browser');

  await manual.getByTestId('ai-manual-add').click();
  await expect(dialog.getByTestId('ai-runtime-ready')).toContainText('CUDA 13 (NVIDIA) is installed');
  await expect(dialog.getByTestId('ai-manual-download')).toHaveCount(0);
  await expect(dialog.getByTestId('ai-runtime-continue')).toBeEnabled();
});

test('a model download blocked by a TLS-inspecting proxy shows the file, its SHA-256 and how to add it', async ({
  page,
}, testInfo) => {
  await openAppWith(page, '?mockAi=setup&mockAiNet=tls&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  const dialog = page.getByRole('dialog', { name: 'Set up the AI assistant' });
  await dialog.getByTestId('ai-setup-start').click();
  await expect(dialog.getByTestId('ai-setup-error')).toContainText('root certificate');

  const manual = dialog.getByRole('region', { name: 'Download manually' });
  await expect(manual.getByTestId('ai-manual-kind')).toContainText('Certificate problem');
  await expect(manual.getByTestId('ai-manual-file-0')).toContainText('.gguf');
  await expect(manual.getByTestId('ai-manual-sha')).toHaveText(/^[0-9a-f]{64}$/);
  await expect(manual.getByTestId('ai-manual-steps')).toContainText('huggingface.co');
  await expect(manual.getByTestId('ai-manual-steps')).toContainText('Add local GGUF');
  await expectNoSeriousViolations(page, testInfo, 'ai-manual-model', '[role=dialog]');

  await manual.getByTestId('ai-manual-add').click();
  // The file was checked and added; the wizard starts again from the model list.
  await expect(dialog.getByTestId('ai-setup-start')).toBeVisible();
});
