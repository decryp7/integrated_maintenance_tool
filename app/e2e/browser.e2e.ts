import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openAiApp, openChat, rowMenu, runCommand, waitAnswered } from './ai-support';

// The internal browser against the mock backends (src/app/browser/mock-browser-backend.ts and the AI mock):
// search the web for an error (the AI drafts the search text), hand the page to the chat, and agent mode with its
// approvals and Stop in the Browser panel's header. The
// real page (a native child webview) cannot be driven here; its rules and placement are tested in Rust.

test.describe('internal browser', () => {
  test('search an error, add the page to the chat and get a cited answer', async ({ page }, testInfo) => {
    await openAiApp(page);
    const menu = await rowMenu(page);
    await menu.getByRole('menuitem', { name: 'Search the web for this error…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Search the web for this error' });
    const text = dialog.getByLabel('Search for');
    await expect(text).toHaveValue('System.TimeoutException The operation has timed out');
    await expect(dialog.getByTestId('web-search-removed')).toContainText('1 value from your logs');
    await expect(dialog.getByTestId('web-search-source')).toContainText('The AI wrote this search');
    await expectNoSeriousViolations(page, testInfo, 'browser-search-dialog', '[role=dialog]');
    await text.fill('TimeoutException gateway pool');
    await dialog.getByRole('button', { name: 'Search', exact: true }).click();
    await expect(dialog).toHaveCount(0);

    const panel = page.locator('imt-web-panel');
    await expect(panel.getByTestId('web-page')).toContainText('google.com/search?q=TimeoutException+gateway+pool');
    await expectNoSeriousViolations(page, testInfo, 'browser-web-panel', 'imt-web-panel');
    await panel.getByRole('button', { name: 'Add page to AI context' }).click();

    const chat = page.locator('imt-ai-chat-panel');
    await expect(chat.getByTestId('chat-source')).toContainText('untrusted web content');
    await expect(chat.getByRole('textbox', { name: 'Ask about the logs' })).toBeEnabled();
    await ask(chat, 'what does this page say about the timeout?');
    await waitAnswered(chat);
    await expect(chat.getByTestId('ai-answer-0')).toContainText('I used it as reference only');
    await expect(chat.getByTestId('ai-answer-0').locator('.sources')).toContainText('TimeoutException gateway pool');
    await expect(chat.getByTestId('chat-source')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'browser-chat-source', 'imt-ai-chat-panel');
  });

  test('agent mode asks before new sites, highlights data from the logs, and Stop ends it', async ({ page }, testInfo) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await chat.getByTestId('ai-browser-menu').click();
    await page.getByRole('menu', { name: 'Browser for this chat' }).getByRole('menuitemradio', { name: /^Agent/ }).click();
    await expect(chat.getByTestId('ai-browser-menu')).toContainText('Browser: Agent');
    await expect(chat).toContainText('New sites, typing and forms need your approval');

    await ask(chat, 'find out on the web why the gateway times out');
    const first = chat.getByTestId('agent-approval').first();
    await expect(first).toContainText('The AI wants to open google.com');
    await expectNoSeriousViolations(page, testInfo, 'browser-agent-approval', 'imt-ai-chat-panel');
    await first.getByRole('button', { name: 'Allow this site for this chat' }).click();
    await expect(first).toContainText('this site is allowed for this chat');
    await expect(chat.getByTestId('agent-step')).toContainText('Step 1 of 25');

    const second = chat.getByTestId('agent-approval').nth(1);
    await expect(second).toContainText('send data from your logs to learn.example.com');
    await expect(second.locator('mark')).toContainText('WEB02');
    await expect(second.getByRole('button', { name: 'Allow this site for this chat' })).toHaveCount(0);
    await second.getByRole('button', { name: 'Deny' }).click();
    await waitAnswered(chat);
    await expect(chat.getByTestId('ai-answer-0')).toContainText('You declined opening the page');

    // Stop while a step waits: it is denied and agent mode ends; the Browser panel's header showed the AI driving.
    await ask(chat, 'look up the error again');
    await expect(chat.getByTestId('agent-approval').nth(2)).toBeVisible();
    await runCommand(page, 'Web: Open the browser panel');
    await expect(page.locator('imt-web-panel').getByTestId('ai-browsing')).toContainText('AI is browsing');
    await page.locator('imt-web-panel').getByTestId('ai-browsing-stop').click();
    await expect(page.locator('imt-web-panel').getByTestId('ai-browsing')).toHaveCount(0);
    await runCommand(page, 'AI: Open the assistant chat');
    await expect(chat.getByTestId('agent-paused')).toContainText('You stopped the AI');
    await expect(chat.getByTestId('ai-browser-menu')).toContainText('Browser: Off');
  });

  test('suggested links open in the browser only when clicked', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await ask(chat, 'suggest a link for this error');
    await waitAnswered(chat);
    const card = chat.getByTestId('link-card');
    await expect(card).toContainText('https://www.google.com/search?q=System.TimeoutException+gateway');
    await card.getByRole('button', { name: 'Open', exact: true }).click();
    await runCommand(page, 'Web: Open the browser panel');
    await expect(page.locator('imt-web-panel').getByTestId('web-page')).toContainText('google.com/search');
  });
});
