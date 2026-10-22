import { expect, Locator, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { ask, openAiApp, openChat, runCommand, waitAnswered } from './ai-support';

// Images for the local AI (ADR 0025) against the scripted AI mock (src/app/core/mock/mock-ai-vision.ts): attaching
// in the chat, explaining an image, finding its related logs, and the offer of a text-only model.

/** A 1×1 PNG: the webview only passes its bytes on (the engine decodes it; the mock checks the signature). */
const PNG_BASE64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

/** Pastes a PNG file named `name` into the chat's question box, as the clipboard would. */
async function pasteImage(chat: Locator, name: string): Promise<void> {
  await chat.getByRole('textbox', { name: 'Ask about the logs' }).evaluate(
    (el, { b64, fileName }) => {
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const data = new DataTransfer();
      data.items.add(new File([bytes], fileName, { type: 'image/png' }));
      el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
    },
    { b64: PNG_BASE64, fileName: name },
  );
}

async function openImageView(page: Page): Promise<Locator> {
  await runCommand(page, 'AI: Open an image…');
  const view = page.locator('debrief-ai-image-panel');
  await expect(view.getByTestId('image-viewer-img')).toBeVisible();
  return view;
}

test.describe('AI vision', () => {
  test('a pasted screenshot goes with the question and shows with the turn', async ({ page }, testInfo) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await pasteImage(chat, 'dialog.png');
    const chip = chat.getByTestId('ai-attachment');
    await expect(chip).toContainText('dialog.png');
    await expect(chip.getByRole('img', { name: 'dialog.png' })).toHaveAttribute('src', /^data:image\//);
    await expectNoSeriousViolations(page, testInfo, 'ai-vision-attachment', 'debrief-ai-chat-panel');

    await ask(chat, 'What does this dialog say?');
    await waitAnswered(chat);
    await expect(chat.getByTestId('ai-attachment')).toHaveCount(0);
    await expect(chat.getByTestId('ai-turn-images').getByRole('img', { name: 'dialog.png' })).toBeVisible();
    await expect(chat.getByTestId('ai-answer-0')).toContainText('error dialog');
  });

  test('an attached image finds its related logs from the chip', async ({ page }) => {
    await openAiApp(page);
    const chat = await openChat(page);
    await chat.getByRole('button', { name: 'Attach an image' }).click();
    const chip = chat.getByTestId('ai-attachment');
    await expect(chip).toContainText('shared.png');
    await chip.getByRole('button', { name: 'Find related logs' }).click();
    await waitAnswered(chat);
    const answer = chat.getByTestId('ai-answer-0');
    await expect(answer).toContainText('Linked events:');
    await answer.locator('debrief-ai-rich-text button.cite.rec').first().click();
    await expect(page.locator('debrief-events-grid .row.sel .lvl')).toContainText('FTL');
  });

  test('the image view explains an image and lists its related logs with their reasons', async ({ page }, testInfo) => {
    await openAiApp(page);
    const view = await openImageView(page);
    await expect(view.getByTestId('image-facts')).toContainText('original 2732 × 1536 PNG');

    await view.getByRole('button', { name: 'Explain with AI' }).click();
    await expect(view.getByTestId('ai-image-explanation').locator('debrief-ai-rich-text')).toContainText('error dialog', { timeout: 15_000 });

    await view.getByRole('button', { name: 'Find related logs' }).click();
    const links = view.getByTestId('ai-related-link');
    await expect(links.first()).toBeVisible({ timeout: 15_000 });
    await expect(links.first().getByTestId('ai-related-reason').first()).toContainText(/^time [+-]/);
    await expect(links.first()).toContainText('title matches Application OrderService');
    await expect(view.getByTestId('ai-related-time')).toContainText('shown on the screen');
    await expectNoSeriousViolations(page, testInfo, 'ai-vision-related', 'debrief-ai-image-panel');

    await links.first().getByRole('button', { name: /^Open in grid \(event \d+\)$/ }).click();
    await expect(page.locator('debrief-events-grid .row.sel .lvl')).toContainText('FTL');
  });

  test('an Explorer image whose clocks disagree offers the clock correction', async ({ page }) => {
    await openAiApp(page, `mockOpen=${encodeURIComponent('/shots/clock-skew.png')}`);
    const sidebar = page.locator('debrief-sidebar');
    await sidebar.getByRole('button', { name: 'Open files' }).click();
    await sidebar.getByRole('button', { name: 'Open image clock-skew.png' }).click();
    const view = page.locator('debrief-ai-image-panel');
    await view.getByRole('button', { name: 'Find related logs' }).click();
    const note = view.getByTestId('ai-related-note-clockDisagreement');
    await expect(note).toContainText('The clocks likely disagree', { timeout: 15_000 });
    await note.getByRole('button', { name: /^Adjust the clock of .+…$/ }).click();
    await expect(page.getByRole('dialog', { name: /^Adjust clock/ })).toBeVisible();
  });

  test('a text-only model offers its image support and never downloads by itself', async ({ page }, testInfo) => {
    await openAiApp(page, 'mockAiVision=missing');
    const chat = await openChat(page);
    await chat.getByRole('button', { name: 'Attach an image' }).click();
    const offer = chat.getByTestId('ai-no-vision');
    await expect(offer).toContainText('reads text only');
    await expect(offer.getByRole('button', { name: 'Add image support (640 MB)' })).toBeVisible();
    await expect(offer.getByRole('button', { name: 'Choose a model that reads images…' })).toBeVisible();
    await expect(chat.getByTestId('ai-attachment')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'ai-vision-no-vision', 'debrief-ai-chat-panel');
    // Nothing was downloaded: no projector job ran.
    await expect(offer.getByTestId('ai-projector-running')).toHaveCount(0);
    await expect(page.getByTestId('notice-toast').filter({ hasText: 'image support' })).toHaveCount(0);

    await offer.getByRole('button', { name: 'Choose a model that reads images…' }).click();
    const models = page.getByRole('dialog', { name: 'AI models' });
    await expect(models.getByTestId('ai-installed-Qwen3.5-4B-Q4_K_M.gguf')).toContainText('Text only');
  });
});
