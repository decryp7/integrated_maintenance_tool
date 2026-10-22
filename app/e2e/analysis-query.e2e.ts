import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openQuery, runCommand } from './analysis-support';

test.describe('query bar', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('suggests completions and accepts one with the keyboard', async ({ page }, testInfo) => {
    const input = await openQuery(page);
    await input.click();
    await input.pressSequentially('level >= warning | stats count() by lev');
    const options = page.locator('imt-query-field').getByRole('option');
    await expect(options.first()).toContainText('level');
    await expect(input).toHaveAttribute('aria-expanded', 'true');
    await expectNoSeriousViolations(page, testInfo, 'query-completions', 'imt-query-field');
    await page.keyboard.press('Tab');
    await expect(input).toHaveValue('level >= warning | stats count() by level');
    await expect(options).toHaveCount(0);
  });

  test('underlines the error the engine reports and explains it', async ({ page }) => {
    const input = await openQuery(page);
    await input.click();
    await input.pressSequentially('level >= warning | frob');
    const underline = page.locator('imt-query-field .mirror .err');
    await expect(underline).toHaveText('frob');
    await expect(input).toHaveAttribute('aria-invalid', 'true');
    await expect(page.locator('#ql-error')).toContainText("Unknown stage 'frob' at column 20");
    await input.fill('level >= warning');
    await expect(underline).toHaveCount(0);
    await expect(input).not.toHaveAttribute('aria-invalid', 'true');
  });

  test('Ctrl+Enter applies a filtering query to the workspace and clears it again', async ({ page }) => {
    const input = await openQuery(page);
    await input.click();
    await input.fill('level >= error');
    await input.press('ControlOrMeta+Enter');
    const applied = page.locator('[data-testid=query-applied]');
    await expect(applied).toContainText('Filtering with');
    await expect(applied).toContainText('level >= error');
    await expect(applied).toContainText(/\d+ events/);
    await applied.getByRole('button', { name: 'Clear' }).click();
    await expect(applied).toHaveCount(0);
  });

  test('a stats query shows its table in Query results, sortable, with CSV export', async ({ page }, testInfo) => {
    const input = await openQuery(page);
    await input.click();
    await input.fill('| stats count() by level');
    await input.press('ControlOrMeta+Enter');
    const results = page.locator('imt-query-results-view');
    await expect(results).toBeVisible();
    await expect(results.getByRole('columnheader', { name: 'level' })).toBeVisible();
    await expect(results.getByRole('columnheader', { name: 'count()' })).toBeVisible();
    const rows = results.locator('tbody tr');
    await expect(rows.nth(1)).toBeVisible();

    await results.getByRole('button', { name: /count\(\)/ }).click();
    await expect(results.getByRole('columnheader', { name: /count\(\)/ })).toHaveAttribute('aria-sort', 'ascending');
    const counts = (await rows.locator('td:nth-child(2)').allInnerTexts()).map(Number);
    expect(counts).toEqual([...counts].sort((a, b) => a - b));

    const download = page.waitForEvent('download');
    await results.getByRole('button', { name: 'Export CSV' }).click();
    expect((await download).suggestedFilename()).toBe('query-result.csv');
    await expectNoSeriousViolations(page, testInfo, 'query-results', 'imt-query-results-view');
  });

  test('history lists successful queries', async ({ page }) => {
    const input = await openQuery(page);
    await input.click();
    await input.fill('level = fatal');
    await input.press('ControlOrMeta+Enter');
    await expect(page.locator('[data-testid=query-applied]')).toBeVisible();
    await input.fill('');
    await page.getByRole('button', { name: 'Query history' }).click();
    await page.getByRole('button', { name: 'level = fatal' }).click();
    await expect(input).toHaveValue('level = fatal');
  });

  test('the palette command focuses the query bar', async ({ page }) => {
    await runCommand(page, 'Query: focus the search field');
    await expect(page.getByTestId('query-field')).toBeFocused();
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`has no serious accessibility violations (${scheme})`, async ({ page }, testInfo) => {
      await page.emulateMedia({ colorScheme: scheme });
      const input = await openQuery(page);
      await input.click();
      await input.pressSequentially('level >= warning | stats count() by source');
      await expectNoSeriousViolations(page, testInfo, `query-${scheme}`, 'imt-query-field');
    });
  }
});
