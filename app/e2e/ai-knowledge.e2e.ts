import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// Settings ▸ AI ▸ Knowledge (ADR 0029) against the browser build's mock knowledge backend
// (src/app/ai/knowledge/mock-knowledge-backend.ts): write a project file, it is listed and read by the AI; disable it;
// remove it after the confirmation.

const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });

async function openKnowledge(page: Page) {
  await page.keyboard.press('ControlOrMeta+Comma');
  await expect(settingsDialog(page)).toBeVisible();
  await settingsDialog(page).getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  const project = settingsDialog(page).getByTestId('knowledge-project');
  await expect(project).toBeVisible();
  return project;
}

test('a knowledge file is added enabled, disabled and removed', async ({ page }, testInfo) => {
  // Four dialogs and an accessibility scan: slow on WebKit under a parallel suite.
  test.slow();
  await openApp(page);
  const project = await openKnowledge(page);
  await expect(project).toContainText('No knowledge files yet.');

  await project.getByRole('button', { name: 'New…' }).click();
  const editor = page.getByRole('dialog', { name: 'New knowledge file' });
  await editor.getByTestId('knowledge-name').fill('Glossary');
  await editor.getByTestId('knowledge-text').fill('# Names\n\n- **ACME-GW**: the payment gateway');
  await editor.getByTestId('knowledge-preview-tab').click();
  await expect(editor.getByTestId('knowledge-preview').locator('strong')).toHaveText('ACME-GW');
  await expectNoSeriousViolations(page, testInfo, 'knowledge-editor', '[role=dialog]');
  await editor.getByTestId('knowledge-save').click();
  await expect(editor).toHaveCount(0);

  const file = project.getByTestId('knowledge-file-Glossary');
  const enabled = file.getByRole('checkbox', { name: 'The AI reads Glossary' });
  await expect(enabled).toBeChecked();
  await enabled.uncheck();
  await expect(enabled).not.toBeChecked();
  await expect(file).toContainText('not read by the AI');

  await file.getByRole('button', { name: 'Remove Glossary' }).click();
  const confirm = page.getByRole('dialog', { name: 'Remove knowledge file?' });
  await expect(confirm.getByTestId('confirm-message')).toContainText('Remove Glossary from this project?');
  await confirm.getByRole('button', { name: 'Remove' }).click();
  await expect(confirm).toHaveCount(0);
  await expect(project).toContainText('No knowledge files yet.');
});
