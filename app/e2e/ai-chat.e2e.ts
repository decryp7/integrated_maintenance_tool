import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openAiApp, openChat, runCommand, waitAnswered } from './ai-support';
import { openMenu } from './menu-support';
import { hideInspectorFlyout } from './app';

// The assistant chat against the scripted AI mock (src/app/core/mock/mock-ai-backend.ts): streaming with tool
// chips, evidence links, and proposals applied through the normal save paths.

test.describe('AI chat', () => {
  test('streams an answer with a tool chip, and a cited event jumps to the grid', async ({ page }, testInfo) => {
    await openAiApp(page);
    const chat = await openChat(page);
    // A new chat offers starter questions.
    await expect(chat.getByRole('list', { name: 'Suggested questions' }).getByRole('button')).toHaveCount(4);

    await ask(chat, 'What crashed?');
    const chip = chat.getByTestId('ai-tool-chip').first();
    await expect(chip).toContainText('Query: 1 events');
    await waitAnswered(chat);
    const answer = chat.getByTestId('ai-answer-0');
    await expect(answer).toContainText('OutOfMemoryException');
    await expect(answer.locator('pre')).toContainText('System.OutOfMemoryException');
    await expectNoSeriousViolations(page, testInfo, 'ai-chat-answer', 'imt-ai-chat-panel');

    await answer.locator('imt-ai-rich-text button.cite.rec').first().click();
    const selected = page.locator('imt-events-grid .row.sel');
    await expect(selected).toBeVisible();
    await expect(selected.locator('.lvl')).toContainText('FTL');
  });

  test('the context chip lists what is sent and can be toggled; chats are created and exported', async ({ page }) => {
    await openAiApp(page);
    await page.locator('imt-events-grid [role=row].row').first().click();
    await hideInspectorFlyout(page);
    const chat = await openChat(page);
    const chip = chat.getByTestId('ai-context-chip');
    await expect(chip).toContainText('selected');
    await chip.click();
    await expect(chip).toHaveAttribute('aria-expanded', 'true');
    const selected = chat.getByRole('checkbox', { name: /^Selected: 1 event/ });
    await expect(selected).toBeChecked();
    await selected.uncheck();
    await expect(chip).not.toContainText('selected');

    await ask(chat, 'What crashed?');
    await waitAnswered(chat);
    await chat.getByRole('button', { name: 'Export…' }).click();
    await expect(chat.getByRole('status').filter({ hasText: 'Exported to' })).toBeVisible();
    // The same path again: the overwrite guard asks first.
    await chat.getByRole('button', { name: 'Export…' }).click();
    const replace = page.getByRole('dialog', { name: 'Replace file?' });
    await expect(replace).toBeVisible();
    await replace.getByRole('button', { name: 'Replace' }).click();
    await expect(replace).toHaveCount(0);

    await chat.getByRole('button', { name: 'New chat' }).click();
    await expect(chat.getByTestId('ai-chat-sessions').locator('option')).toHaveCount(2);
    await expect(chat.getByTestId('ai-answer-0')).toHaveCount(0);
    await chat.getByRole('button', { name: 'Delete this chat' }).click();
    await expect(chat.getByTestId('ai-chat-sessions').locator('option')).toHaveCount(1);
    await expect(chat.getByTestId('ai-answer-0')).toContainText('OutOfMemoryException');
  });

  test('a regex proposal is applied as a highlight rule through the marker dialog', async ({ page }, testInfo) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await ask(chat, 'suggest a regex for these lines');
    const card = chat.getByTestId('ai-proposal');
    await expect(card).toContainText('Matches 2 of 2 examples');
    await expect(card).toContainText('(?<timestamp>');
    await waitAnswered(chat);
    await expectNoSeriousViolations(page, testInfo, 'ai-chat-proposal', 'imt-ai-chat-panel');

    await card.getByRole('button', { name: 'Add as highlight…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Highlight lines' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('textbox', { name: 'Value' })).toHaveValue(/timestamp/);
    await dialog.getByLabel('Label', { exact: true }).fill('AI pattern');
    await dialog.getByRole('button', { name: 'Add highlight' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(card).toContainText('applied');
    await expect(card).toContainText('Highlight rule added.');

    await runCommand(page, 'Manage markers');
    await expect(page.getByRole('table', { name: 'Marker rules' })).toContainText('AI pattern');
  });

  test('a pipeline proposal is added to the profile and can be undone', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await ask(chat, 'drop health checks with a pipeline');
    const card = chat.getByTestId('ai-proposal');
    await expect(card).toContainText('keeps 52, drops 8, changes 0');
    await card.getByRole('button', { name: 'Add to profile' }).click();
    await expect(card).toContainText('Pipeline steps added to the profile');
    await openMenu(page, 'Edit');
    await expect(page.getByRole('menuitem', { name: /^Undo AI: pipeline steps/ })).toBeVisible();
  });
});
