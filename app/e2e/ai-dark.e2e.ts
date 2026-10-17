import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openAiApp, openChat, runCommand, waitAnswered } from './ai-support';
import { openAppWith } from './mock-url';

// Every AI view in the DARK theme (the other ai-* files check the light one): contrast of chips, badges,
// citation links, proposal cards and approval prompts on the dark tokens.
test.use({ colorScheme: 'dark' });

test.describe('AI views in the dark theme', () => {
  test('chat answer, proposal card and approval prompt', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockAiWeb=1');
    const chat = await openChat(page);
    await ask(chat, 'What crashed?');
    await waitAnswered(chat);
    await expect(chat.getByTestId('ai-tool-chip').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-chat-answer', 'imt-ai-chat-panel');

    await ask(chat, 'suggest a regex for these lines');
    await expect(chat.getByTestId('ai-proposal')).toContainText('Matches 2 of 2');
    await waitAnswered(chat);
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-chat-proposal', 'imt-ai-chat-panel');

    await ask(chat, 'search the web for this timeout');
    await expect(chat.getByTestId('ai-approval').last()).toContainText('Search the web for:');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-approval', 'imt-ai-chat-panel');
    await chat.getByTestId('ai-approval').last().getByRole('button', { name: 'Deny' }).click();
    await waitAnswered(chat);
  });

  test('query, narrative and propose dialogs', async ({ page }, testInfo) => {
    await openAiApp(page);
    await runCommand(page, 'AI: Ask AI to write a query…');
    const query = page.getByRole('dialog', { name: 'Ask AI to write a query' });
    await query.getByLabel('What do you want to find?').fill('warnings only');
    await query.getByRole('button', { name: 'Write query' }).click();
    await expect(query.getByTestId('ai-proposal')).toContainText('level = warning');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-query-dialog', '[role=dialog]');
    await query.getByRole('button', { name: 'Close' }).last().click();

    await runCommand(page, 'AI: Find likely root causes');
    const narrative = page.getByRole('dialog', { name: 'Likely root causes' });
    await expect(narrative.getByTestId('ai-narrative')).toContainText('Memory leak');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-narrative', '[role=dialog]');
    await narrative.getByRole('button', { name: 'Close' }).last().click();

    await page.locator('imt-events-grid [role=row].row').first().click();
    await runCommand(page, 'AI: Propose markers…');
    const propose = page.getByRole('dialog', { name: 'Propose markers' });
    await propose.getByLabel('Which events should be marked?').fill('timeouts');
    await propose.getByRole('button', { name: 'Propose', exact: true }).click();
    await expect(propose.getByTestId('ai-proposal')).toContainText('Timeout');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-propose', '[role=dialog]');
  });

  test('setup wizard and model manager', async ({ page }, testInfo) => {
    await openAppWith(page, '?mockAi=setup&mockAiStepMs=5');
    await runCommand(page, 'Set up AI assistant');
    const wizard = page.getByRole('dialog', { name: 'Set up the AI assistant' });
    await expect(wizard.getByTestId('ai-hardware')).toContainText('8 GB video memory');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-setup-plan', '[role=dialog]');
    await wizard.getByTestId('ai-setup-start').click();
    await expect(wizard.getByTestId('ai-setup-ready')).toContainText('Vulkan');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-setup-ready', '[role=dialog]');
    await wizard.getByRole('button', { name: 'Close' }).first().click();

    await runCommand(page, 'AI: Manage models…');
    const models = page.getByRole('dialog', { name: 'AI models' });
    await expect(models).toContainText('Qwen3.5 9B');
    await expectNoSeriousViolations(page, testInfo, 'dark-ai-models', '[role=dialog]');
  });
});
