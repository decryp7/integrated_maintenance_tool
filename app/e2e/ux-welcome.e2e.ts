import { expect, test } from '@playwright/test';
import { openAppWith } from './mock-url';
import { runCommand } from './ai-support';

// Usability review 2026-10-23 (U7): the welcome page is centred and offers (quietly, dismissibly) to set up the AI
// assistant; the activity bar has an entry for it before it is set up. Palette: "AI" finds the set-up near the top.

test.beforeEach(async ({ page }) => {
  // The first-start setup dialog is not what is under test.
  await page.addInitScript(() => {
    localStorage.clear();
    localStorage.setItem('debrief.ai.setupOffered.v1', '1');
  });
});

test('the activity bar offers the AI set-up before a model exists', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiStepMs=5');
  await expect(page.locator('debrief-activity-bar [data-activity-id="ai-setup"]')).toBeVisible();
  await expect(page.locator('debrief-activity-bar [data-activity-id="ai"]')).toHaveCount(0);
});

test('the welcome page is centred and has a dismissible AI offer', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiStepMs=5');
  await runCommand(page, 'Close workspace');
  const offer = page.getByTestId('welcome-ai-offer');
  await expect(offer).toBeVisible();
  const centre = await page.locator('#welcome-title').evaluate((e) => {
    const b = e.getBoundingClientRect();
    const host = document.querySelector('debrief-welcome')!.getBoundingClientRect();
    return Math.abs(b.left + b.width / 2 - (host.left + host.width / 2));
  });
  expect(centre).toBeLessThan(4);
  await page.getByTestId('welcome-ai-dismiss').click();
  await expect(offer).toHaveCount(0);
  // It stays dismissed (stored).
  expect(await page.evaluate(() => localStorage.getItem('debrief.welcome.aiOfferDismissed'))).toBe('1');
});

test('the palette ranks "AI" matches first, without loose ones or duplicates', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup&mockAiStepMs=5');
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill('AI');
  const options = page.locator('debrief-command-palette').getByRole('option');
  await expect(options.first()).toBeVisible();
  const texts = (await options.allInnerTexts()).map((t) => t.replace(/\s+/g, ' ').trim());
  const setUp = texts.findIndex((t) => t.includes('Set up AI assistant'));
  expect(setUp).toBeGreaterThanOrEqual(0);
  expect(setUp).toBeLessThanOrEqual(2);
  expect(texts.some((t) => /Open an image/.test(t) && !/AI/.test(t))).toBe(false);
  // One entry per command: no "Open AI assistant" twin of "AI: Open the assistant chat".
  expect(texts.filter((t) => /assistant chat|Open AI assistant/i.test(t))).toHaveLength(1);
});
