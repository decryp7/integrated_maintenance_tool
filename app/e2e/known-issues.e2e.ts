import { expect, test } from '@playwright/test';
import { runCommand } from './ai-support';
import { expectNoSeriousViolations } from './axe';
import { gridRows, openAppWith } from './mock-url';

// Known-issue matching (ADR 0039) in the Problems pane against the mock backend: the sample knowledge file declares
// "Payment gateway timeouts"; the mock AI suggests "Transactions rolled back after lock timeouts".

async function openPane(page: import('@playwright/test').Page) {
  await runCommand(page, 'Problems: Check known issues');
  const section = page.getByTestId('known-issues');
  await expect(section).toBeVisible();
  return section;
}

test('a declared known issue shows its fix, count and rule origin, and Show events filters the grid', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockAiStepMs=5');
  const section = await openPane(page);
  const card = section.getByTestId('known-issue-payment-gateway-timeouts');
  await expect(card).toContainText('Payment gateway timeouts');
  await expect(card.getByTestId('known-issue-origin')).toHaveText('From rule');
  await expect(card.getByTestId('known-issue-count')).toContainText(/\d+ events/);
  await expect(card.getByTestId('known-issue-fix')).toContainText('Raise the gateway timeout');
  await expect(section.getByTestId('known-issues-warnings')).toContainText('1 knowledge notes need fixing');

  const before = await gridRows(page).count();
  await card.getByRole('button', { name: 'Show events' }).click();
  await expect(gridRows(page).first()).toBeVisible();
  await expect(page.locator('debrief-status-bar')).toContainText('events');
  await expect.poll(async () => gridRows(page).count()).toBeLessThanOrEqual(before);
  await expect(gridRows(page).first()).toContainText(/timed out/i);
  await expectNoSeriousViolations(page, testInfo, 'known-issues', 'debrief-known-issues');
});

test('Suggest with AI lists engine-verified suggestions with the AI badge', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockAiStepMs=5');
  const section = await openPane(page);
  await section.getByRole('button', { name: 'Suggest with AI' }).click();
  const card = section.getByTestId('known-issue-ai-lock-rollbacks');
  await expect(card).toBeVisible();
  await expect(card.getByTestId('known-issue-origin')).toHaveText('AI suggested, verified by the engine');
  await expectNoSeriousViolations(page, testInfo, 'known-issues-ai', 'debrief-known-issues');
});

test('without an AI model the suggest button is disabled and says what to do', async ({ page }) => {
  await openAppWith(page, '?mockAi=setup');
  const section = await openPane(page);
  const button = section.getByRole('button', { name: 'Suggest with AI' });
  await expect(button).toBeDisabled();
  await expect(button).toHaveAttribute('title', 'Needs an AI model: set one up in Settings > AI');
  await expect(section.getByTestId('known-issue-payment-gateway-timeouts')).toBeVisible();
});
