import { expect, test } from '@playwright/test';
import { gridTotal, hideInspectorFlyout, parseCount } from './app';
import { openQuery } from './analysis-support';
import { expectNoSeriousViolations } from './axe';
import { narrativeDialog, openAiApp, rowMenu, runCommand, viewAnswer } from './ai-support';
import { menuItem, openMenu, showProfileEditor } from './menu-support';

// The AI entry points outside the chat (query bar, context menu, Overview, Moment, markers manager, profile
// editor, infer-regex dialog) and AI being absent when it is unavailable. Scripted AI mock.

test.describe('AI entry points', () => {
  test('Ask AI writes a query from plain words and Apply runs it', async ({ page }, testInfo) => {
    await openAiApp(page);
    const before = parseCount(await gridTotal(page).innerText());
    await openQuery(page);
    await page.getByTestId('ai-ask-query').click();
    const dialog = page.getByRole('dialog', { name: 'Ask AI to write a query' });
    await dialog.getByLabel('What do you want to find?').fill('warnings only');
    await dialog.getByRole('button', { name: 'Write query' }).click();
    const card = dialog.getByTestId('ai-proposal');
    await expect(card).toContainText('level = warning');
    await expect(card).toContainText('37 results now');
    await expectNoSeriousViolations(page, testInfo, 'ai-query-dialog', '[role=dialog]');
    await card.getByRole('button', { name: 'Apply query' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.locator('[data-testid=query-applied]')).toContainText('level = warning');
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(before);
  });

  test('Explain this event from the context menu, with a citation that selects the event', async ({ page }, testInfo) => {
    await openAiApp(page);
    const menu = await rowMenu(page);
    await menu.getByRole('menuitem', { name: 'Explain this event (AI)…' }).click();
    // It runs in the background: a toast says so, then "Explanation ready · View" opens the answer.
    await expect(page.getByTestId('notice-toast').filter({ hasText: /Explaining event #\d+ — running in Jobs/ })).toBeVisible();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await viewAnswer(page, 'Explanation ready');
    const dialog = page.getByRole('dialog', { name: /^Explain event \d+$/ });
    const answer = dialog.getByTestId('ai-narrative');
    await expect(answer).toContainText('payment gateway timed out');
    await expectNoSeriousViolations(page, testInfo, 'ai-narrative', '[role=dialog]');
    await answer.locator('button.cite.rec').first().click();
    await expect(dialog).toHaveCount(0);
    await expect(page.locator('imt-events-grid .row.sel')).toBeVisible();
  });

  test('Root causes from Home and Summarize from the Moment view', async ({ page }) => {
    await openAiApp(page);
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    // Review on the timeline header's headline opens Home.
    await page.getByRole('toolbar', { name: 'Health check' }).getByRole('button', { name: 'Review' }).click({ timeout: 30_000 });
    const home = page.locator('imt-home-view');
    await expect(home).toBeVisible({ timeout: 10_000 });
    await home.getByTestId('ai-root-causes').click();
    await viewAnswer(page, 'Root causes ready');
    const causes = narrativeDialog(page);
    await expect(causes.getByTestId('ai-narrative')).toContainText('Memory leak on WEB02');
    await causes.getByRole('button', { name: 'Close', exact: true }).click();

    await runCommand(page, 'Analyse this moment');
    const moment = page.locator('imt-moment-view');
    await expect(moment).toBeVisible();
    await moment.getByTestId('ai-summarize').click();
    await viewAnswer(page, 'Summary ready');
    await expect(narrativeDialog(page).getByTestId('ai-narrative')).toContainText('WEB02 ran out of memory');
  });

  test('Propose markers from the markers manager, and pipeline changes from the profile editor', async ({ page }, testInfo) => {
    await openAiApp(page);
    await page.locator('imt-events-grid [role=row].row').first().click();
    await hideInspectorFlyout(page);
    await runCommand(page, 'Manage markers');
    await page.getByTestId('ai-propose-markers').click();
    const dialog = page.getByRole('dialog', { name: 'Propose markers' });
    await expect(dialog).toContainText('the lines around event');
    await dialog.getByLabel('Which events should be marked?').fill('timeouts');
    await dialog.getByRole('button', { name: 'Propose' }).click();
    const card = dialog.getByTestId('ai-proposal');
    await expect(card).toContainText('Timeout');
    await expectNoSeriousViolations(page, testInfo, 'ai-propose-dialog', '[role=dialog]');
    await card.getByRole('button', { name: 'Add to profile' }).click();
    await expect(card).toContainText('Markers added to the profile');
    await dialog.getByRole('button', { name: 'Close' }).last().click();
    await expect(page.getByRole('table', { name: 'Marker rules' })).toContainText('Timeout');

    await showProfileEditor(page);
    await page.getByTestId('ai-profile-propose').click();
    const changes = page.getByRole('dialog', { name: 'Propose profile changes' });
    await changes.getByLabel('Propose', { exact: true }).selectOption('extractors');
    await changes.getByLabel('Which values should be extracted?').fill('durations');
    await changes.getByRole('button', { name: 'Propose', exact: true }).click();
    await expect(changes.getByTestId('ai-proposal')).toContainText('elapsed');
  });

  test('Suggest with AI fills the infer-regex dialog', async ({ page }) => {
    await openAiApp(page);
    const menu = await rowMenu(page);
    await menu.getByRole('menuitem', { name: 'Infer pattern from lines…' }).click();
    const infer = page.getByRole('dialog', { name: /Infer pattern/ });
    await expect(infer.getByLabel('Pattern (editable)')).not.toHaveValue('');
    await infer.getByTestId('ai-suggest-regex').click();
    const suggest = page.getByRole('dialog', { name: 'Suggest a regex' });
    await suggest.getByRole('button', { name: 'Suggest regex' }).click();
    await suggest.getByRole('button', { name: 'Use this pattern' }).click();
    await expect(suggest).toHaveCount(0);
    await expect(infer.getByLabel('Pattern (editable)')).toHaveValue(/\(\?<timestamp>/);
  });

  test('without AI in this run, its entry points are absent', async ({ page }) => {
    await openAiApp(page, 'mockAi=off');
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await input.fill('AI:');
    await expect(page.locator('imt-command-palette').getByRole('option', { name: /AI: Open the assistant chat/ })).toHaveCount(0);
    await page.keyboard.press('Escape');

    const view = await openMenu(page, 'View');
    await expect(menuItem(view, 'AI')).toHaveCount(0);
    await page.keyboard.press('Escape');

    const menu = await rowMenu(page);
    await expect(menu.getByRole('menuitem', { name: /Infer pattern/ })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: /\(AI\)/ })).toHaveCount(0);
    await page.keyboard.press('Escape');

    await openQuery(page);
    await expect(page.getByTestId('query-field')).toBeVisible();
    await expect(page.getByTestId('ai-ask-query')).toHaveCount(0);
  });
});
