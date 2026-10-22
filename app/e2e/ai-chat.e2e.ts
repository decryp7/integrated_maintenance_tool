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

  test('the Context menu chooses what is sent; chats are created and exported', async ({ page }, testInfo) => {
    await openAiApp(page);
    await page.locator('imt-events-grid [role=row].row').first().click();
    await hideInspectorFlyout(page);
    const chat = await openChat(page);
    const context = chat.getByTestId('ai-context-menu');
    await expect(context).toContainText('Context: Selected events (1)');
    // Keyboard reachable: Enter on the button opens the menu with its first item focused.
    await context.focus();
    await page.keyboard.press('Enter');
    const menu = page.getByRole('menu', { name: 'Context sent with your question' });
    await expect(menu.getByRole('menuitemradio', { name: /^Selected events \(1\)/ })).toHaveAttribute('aria-checked', 'true');
    await expect(menu).toContainText('Nothing leaves this computer.');
    await expectNoSeriousViolations(page, testInfo, 'ai-context-menu', '[role=menu]');
    // Keyboard: the menu takes the arrow keys and Enter.
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(menu).toHaveCount(0);
    await expect(context).toContainText('Context: Current view');

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

  test('a long conversation scrolls in its list only: the panel has no extra scroll space and the composer stays at the bottom', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    for (let i = 0; i < 6; i++) {
      await ask(chat, 'What crashed?');
      await waitAnswered(chat);
    }
    const log = chat.getByRole('log', { name: 'Conversation' });
    expect(await log.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
    // Neither the panel nor any box around it up to the dock can scroll (text for screen readers in the scrolled
    // list once made the dock's view scroll past the panel into empty space).
    const extra = await chat.evaluate((el) => {
      const out: string[] = [];
      for (let n: HTMLElement | null = el; n && !n.classList.contains('dv-dockview'); n = n.parentElement) {
        if (n.scrollHeight > n.clientHeight + 1) out.push(`${n.tagName.toLowerCase()}.${n.className}: ${n.scrollHeight} > ${n.clientHeight}`);
      }
      return out;
    });
    expect(extra).toEqual([]);
    // Every panel container clips: it cannot be scrolled, even programmatically (focus, scrollIntoView).
    const scrolled = await page.evaluate(() =>
      Array.from(document.querySelectorAll<HTMLElement>('.imt-panel')).filter((el) => {
        el.scrollTop = 10_000;
        return el.scrollTop !== 0;
      }).length,
    );
    expect(scrolled).toBe(0);
    const panel = (await chat.boundingBox())!;
    const composer = (await chat.locator('imt-chat-composer').boundingBox())!;
    expect(Math.abs(composer.y + composer.height - (panel.y + panel.height))).toBeLessThan(2);
  });
});
