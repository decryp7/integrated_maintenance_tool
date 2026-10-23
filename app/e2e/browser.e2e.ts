import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openAiApp, openChat, rowMenu, waitAnswered } from './ai-support';

// The AI's browser against the mock backends (src/app/browser/mock-browser-backend.ts and the AI mock): search the web
// for an error (the AI drafts the search text; the search opens in the default browser), the composer's Browser modes,
// "Add the open page to the question", agent mode with its approvals, and the status chip with its Stop. The real
// Chromium window cannot be driven here; the DevTools driver is tested in Rust.

test.describe('AI browser', () => {
  test('search an error opens the search in the default browser; the open page is added to the chat and cited', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockPage=open');
    const menu = await rowMenu(page);
    await menu.getByRole('menuitem', { name: 'Search the web for this error…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Search the web for this error' });
    const text = dialog.getByLabel('Search for');
    await expect(text).toHaveValue('System.TimeoutException The operation has timed out');
    await expect(dialog.getByTestId('web-search-removed')).toContainText('1 value from your logs');
    await expect(dialog.getByTestId('web-search-source')).toContainText('The AI wrote this search');
    await expect(dialog).toContainText('in your default browser');
    await expectNoSeriousViolations(page, testInfo, 'browser-search-dialog', '[role=dialog]');
    await text.fill('TimeoutException gateway pool');
    await dialog.getByRole('button', { name: 'Search', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    // No panel opens: the page belongs to the user's own browser.
    await expect(page.locator('debrief-web-panel')).toHaveCount(0);

    const chat = await openChat(page);
    await expect(chat.getByTestId('chat-capture')).toHaveCount(0);
    await chat.getByTestId('ai-browser-menu').click();
    await page.getByRole('menu', { name: 'Browser for this chat' }).getByRole('menuitemradio', { name: /^Read along/ }).click();
    await expect(chat.getByTestId('ai-browser-menu')).toContainText('Browser: Read along');
    await chat.getByRole('button', { name: 'Add the open page to the question' }).click();
    await expect(chat.getByTestId('chat-source')).toContainText('untrusted web content');
    await ask(chat, 'what does this page say about the timeout?');
    await waitAnswered(chat);
    await expect(chat.getByTestId('ai-answer-0')).toContainText('I used it as reference only');
    await expect(chat.getByTestId('chat-source')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'browser-chat-source', 'debrief-ai-chat-panel');
  });

  test('adding the open page says so when no page is open', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await chat.getByTestId('ai-browser-menu').click();
    await page.getByRole('menu', { name: 'Browser for this chat' }).getByRole('menuitemradio', { name: /^Read along/ }).click();
    await chat.getByRole('button', { name: 'Add the open page to the question' }).click();
    await expect(chat.getByRole('alert')).toContainText('No page is open');
  });

  test('the composer switches Off, Read along and Agent, and Agent says a real window will open', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    const menu = chat.getByTestId('ai-browser-menu');
    await expect(menu).toContainText('Browser: Off');
    await menu.click();
    const list = page.getByRole('menu', { name: 'Browser for this chat' });
    await expect(list.getByRole('menuitemradio', { name: /^Agent/ })).toContainText('A real browser window will open next to the app');
    await list.getByRole('menuitemradio', { name: /^Agent/ }).click();
    await expect(menu).toContainText('Browser: Agent');
    await expect(chat).toContainText('A real browser window will open next to the app');
    await menu.click();
    await page.getByRole('menu', { name: 'Browser for this chat' }).getByRole('menuitemradio', { name: /^Read along/ }).click();
    await expect(menu).toContainText('Browser: Read along');
    await menu.click();
    await page.getByRole('menu', { name: 'Browser for this chat' }).getByRole('menuitemradio', { name: /^Off/ }).click();
    await expect(menu).toContainText('Browser: Off');
    await expect(chat.getByTestId('chat-capture')).toHaveCount(0);
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
    await expectNoSeriousViolations(page, testInfo, 'browser-agent-approval', 'debrief-ai-chat-panel');
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

    // Stop while a step waits: the status chip shows the AI driving; its Stop denies the step and ends agent mode.
    await ask(chat, 'look up the error again');
    await expect(chat.getByTestId('agent-approval').nth(2)).toBeVisible();
    const chip = page.locator('debrief-status-bar').getByTestId('ai-browsing');
    await expect(chip).toBeVisible();
    await expect(chip).toHaveAttribute('role', 'status');
    await expectNoSeriousViolations(page, testInfo, 'browser-status-chip', 'debrief-status-bar');
    await chip.getByRole('button', { name: "Stop the AI's browser" }).click();
    await expect(chip).toHaveCount(0);
    await expect(chat.getByTestId('agent-paused')).toContainText('You stopped the AI');
    await expect(chat.getByTestId('ai-browser-menu')).toContainText('Browser: Off');
  });

  test('suggested links open in the default browser only when clicked', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await ask(chat, 'suggest a link for this error');
    await waitAnswered(chat);
    const card = chat.getByTestId('link-card');
    await expect(card).toContainText('https://www.google.com/search?q=System.TimeoutException+gateway');
    await card.getByRole('button', { name: 'Open', exact: true }).click();
    await expect(chat.getByRole('alert')).toHaveCount(0);
    await expect(page.locator('debrief-web-panel')).toHaveCount(0);
  });

  test('the status chip shows research, then Stop cancels it', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockDriving=research');
    const chip = page.locator('debrief-status-bar').getByTestId('ai-browsing');
    await expect(chip).toBeVisible();
    await expect(chip).toHaveAttribute('aria-live', 'polite');
    await expectNoSeriousViolations(page, testInfo, 'browser-status-chip-research', 'debrief-status-bar');
    await chip.getByRole('button', { name: "Stop the AI's browser" }).click();
    await expect(chip).toHaveCount(0);
  });
});
