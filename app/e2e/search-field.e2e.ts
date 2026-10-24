import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';

// The filter bar's ONE search field: plain words search the messages live; text with query structure is a query that
// runs with Enter or Ctrl/Cmd+Enter. The mock backend classifies like the engine (operator, pipe, exists(), known field).

const field = (page: Page) => page.getByTestId('query-field');
const status = (page: Page) => page.getByTestId('query-status');
const box = (page: Page) => page.locator('debrief-query-field .box');

test.describe('search field', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
  });

  test('is one combobox in the filter bar; there is no separate Query panel', async ({ page }) => {
    await expect(field(page)).toHaveAttribute('role', 'combobox');
    await expect(field(page)).toHaveAccessibleName('Search messages and exceptions, or type a query');
    await expect(page.getByRole('tab', { name: 'Query', exact: true })).toHaveCount(0);
    await expect(page.getByTestId('query-run')).toHaveCount(0);
    await expect(status(page)).toHaveCount(0);
  });

  test('plain words search the messages live, with no Run button or status line', async ({ page }) => {
    const all = await initialTotal(page);
    await field(page).click();
    await field(page).pressSequentially('timed out');
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(parseCount(all));
    await expect(page.getByTestId('query-run')).toHaveCount(0);
    await expect(status(page)).toHaveCount(0);
    await expect(page.locator('debrief-query-field')).not.toHaveClass(/\bquery\b/);
    await field(page).fill('');
    await expect(gridTotal(page)).toHaveText(all);
  });

  test('text with an operator is a query: Run appears, nothing filters until Enter runs it', async ({ page }, testInfo) => {
    const all = await initialTotal(page);
    await field(page).click();
    await field(page).fill('level >= error');
    const run = page.getByTestId('query-run');
    await expect(run).toBeVisible();
    await expect(page.locator('debrief-query-field')).toHaveClass(/\bquery\b/);
    await expect(gridTotal(page)).toHaveText(all);
    await field(page).press('Enter');
    await expect(status(page)).toContainText('Filtering with');
    await expect(status(page)).toContainText('level >= error');
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(parseCount(all));
    await expectNoSeriousViolations(page, testInfo, 'search-field-applied', 'debrief-filter-bar');
    await status(page).getByRole('button', { name: 'Clear' }).click();
    await expect(field(page)).toHaveValue('');
    await expect(gridTotal(page)).toHaveText(all);
    await expect(status(page)).toHaveCount(0);
  });

  test('a pipe, exists() and a known field with an operator are queries; a quoted phrase and a bare word are not', async ({ page }) => {
    const query = page.locator('debrief-query-field.query');
    await field(page).click();
    for (const text of ['| stats count() by level', 'exists(MachineName)', 'source = checkout']) {
      await field(page).fill(text);
      await expect(query).toHaveCount(1);
    }
    for (const text of ['checkout', '"level >= error', "it's broken"]) {
      await field(page).fill(text);
      await expect(query).toHaveCount(0);
    }
  });

  test('Ctrl/Cmd+Enter runs a query and a stats query opens its table', async ({ page }) => {
    await field(page).click();
    await field(page).fill('| stats count() by level');
    await field(page).press('ControlOrMeta+Enter');
    await expect(page.locator('debrief-query-results-view')).toBeVisible();
    await expect(page.locator('debrief-query-results-view').getByRole('columnheader', { name: 'level' })).toBeVisible();
  });

  test('a syntax error shows in the status line with its column, and did-you-mean offers a fix', async ({ page }, testInfo) => {
    await field(page).click();
    await field(page).pressSequentially('level >= warning | frob');
    await expect(status(page)).toContainText("Unknown stage 'frob' at column 20");
    await expect(field(page)).toHaveAttribute('aria-invalid', 'true');
    await field(page).fill('MachinName = WEB01 | stats count()');
    await expect(page.getByTestId('did-you-mean').getByRole('button', { name: 'Replace with MachineName' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'search-field-error', 'debrief-filter-bar');
  });

  test('the field grows to several lines while focused (Shift+Enter) and shrinks when it loses focus', async ({ page }) => {
    await field(page).click();
    const oneLine = (await box(page).boundingBox())!.height;
    await field(page).fill('level >= warning');
    await field(page).press('Shift+Enter');
    await field(page).pressSequentially('| stats count() by level');
    await expect.poll(async () => (await box(page).boundingBox())!.height).toBeGreaterThan(oneLine + 10);
    await expect(box(page)).toHaveClass(/expanded/);
    await page.getByRole('grid', { name: 'Events' }).focus();
    await expect.poll(async () => (await box(page).boundingBox())!.height).toBeLessThanOrEqual(oneLine + 2);
  });

  test('with the Explorer closed the field is at least 300 px wide', async ({ page }) => {
    await page.keyboard.press('ControlOrMeta+b');
    await expect(page.locator('debrief-sidebar')).toHaveCount(0);
    expect((await page.locator('debrief-query-field').boundingBox())!.width).toBeGreaterThanOrEqual(300);
  });

  // At 1440 px with the Explorer open the field is narrow (the filter bar stays one row): its inline controls pack
  // tighter so the text area keeps a usable width beside them and the "+N" partial-match note.
  test('keeps a usable text area: the inline buttons and the partial-match note do not squeeze it out', async ({ page }) => {
    await field(page).fill('imeout');
    await expect(page.getByTestId('partial-note')).toBeVisible();
    expect((await field(page).boundingBox())!.width).toBeGreaterThan(100);
  });

  test('Ctrl/Cmd+/ and the palette focus the field', async ({ page }) => {
    await page.getByRole('grid', { name: 'Events' }).focus();
    await page.keyboard.press('ControlOrMeta+/');
    await expect(field(page)).toBeFocused();
    await page.getByRole('grid', { name: 'Events' }).focus();
    await runCommand(page, 'Query: focus the search field');
    await expect(field(page)).toBeFocused();
  });

  test('history lists the queries that ran, and Save stores the search as a saved query', async ({ page }, testInfo) => {
    const history = page.getByTestId('query-history');
    await expect(history).toBeDisabled();
    await field(page).click();
    await field(page).fill('level = fatal');
    await field(page).press('Enter');
    await expect(status(page)).toContainText('Filtering with');
    await expect(history).toBeEnabled();
    await field(page).fill('');
    await history.click();
    await expectNoSeriousViolations(page, testInfo, 'search-field-history', 'debrief-filter-bar');
    await page.getByRole('button', { name: 'level = fatal', exact: true }).click();
    await expect(field(page)).toHaveValue('level = fatal');

    await expect(page.getByTestId('query-save')).toBeEnabled();
    await page.getByTestId('query-save').click();
    await expect(page.locator('[data-toast]').filter({ hasText: /Saved as "/ })).toBeVisible();
  });

  test('Save is disabled while the field is empty', async ({ page }) => {
    await expect(page.getByTestId('query-save')).toBeDisabled();
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`has no serious accessibility violations with a query typed (${scheme})`, async ({ page }, testInfo) => {
      await page.emulateMedia({ colorScheme: scheme });
      await field(page).click();
      await field(page).pressSequentially('level >= warning | stats count() by source');
      await expectNoSeriousViolations(page, testInfo, `search-field-${scheme}`, 'debrief-filter-bar');
    });
  }
});
