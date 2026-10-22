import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAiApp } from './ai-support';

/** Parsers view ▸ Generate with AI… (ADR 0032), with the scripted AI mock: the engine's verdict is shown. */
test.describe('AI parser generation', () => {
  test('generates a parser for a file of the window and shows the score, the lines not read and the rounds', async ({ page }, testInfo) => {
    await openAiApp(page);
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('Manage parsers');
    await page.keyboard.press('Enter');
    const parsers = page.locator('debrief-parsers-view');
    await parsers.getByRole('button', { name: 'Generate with AI…' }).click();
    await page.getByRole('menu', { name: 'Generate a parser for a file of this window' }).getByRole('menuitem').first().click();

    const propose = page.getByRole('dialog', { name: 'Propose a parser' });
    await expect(propose).toContainText('The engine checks each draft on it');
    await propose.getByRole('button', { name: 'Propose', exact: true }).click();
    const verdict = propose.getByTestId('parser-verdict');
    await expect(verdict).toBeVisible({ timeout: 15_000 });
    await expect(verdict.getByTestId('parser-score')).toHaveText('99.5%');
    await expect(verdict.getByTestId('parser-status')).toHaveText('target reached');
    await expect(verdict.getByTestId('parser-zone')).toContainText("+02:00 from the file's modification time");
    await expect(verdict).toContainText('RequestId dropped');
    await expect(verdict.getByRole('list', { name: 'Rounds' }).getByRole('listitem')).toHaveCount(2);
    await verdict.getByText(/lines not read/).click();
    await expect(verdict.getByRole('table')).toContainText('-- worker paused --');
    await expectNoSeriousViolations(page, testInfo, 'ai-parser-verdict', '[role=dialog]');
    await propose.getByRole('button', { name: 'Add to parsers' }).click();
    await expect(propose.getByTestId('ai-proposal')).toContainText(/Added|added/);
  });
});
