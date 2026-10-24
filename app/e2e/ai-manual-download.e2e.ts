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
  await openAppWith(page, `?mockAiRuntime=none&mockAiNet=tls&mockAiStepMs=5&mockOpen=${CUDA.join(',')}`);
  await runCommand(page, 'AI: Manage models');
  const dialog = page.getByRole('dialog', { name: 'AI models' });
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
  await expect(dialog.getByTestId('ai-runtime-cuda-13')).toContainText('Active');
});

test('a model download blocked by a TLS-inspecting proxy ends the background set-up with a message that names the certificate problem and offers Retry', async ({
  page,
}) => {
  await openAppWith(page, '?mockAi=setup&mockAiNet=tls&mockAiStepMs=5');
  await runCommand(page, 'Set up AI assistant');
  await page.getByRole('dialog', { name: 'Set up the AI assistant' }).getByTestId('ai-setup-start').click();
  const failed = page.getByTestId('notification').filter({ hasText: 'could not be set up' });
  await expect(failed).toContainText('root certificate', { timeout: 20_000 });
  await expect(failed.getByRole('button', { name: 'Retry' })).toBeVisible();
  // The manual download (file, SHA-256, steps) is no longer offered after a failed first set-up.
  await expect(page.getByTestId('ai-manual-download')).toHaveCount(0);
});
