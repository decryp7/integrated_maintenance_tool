import { expect, Page, test } from '@playwright/test';
import { expandFields, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';

/** Selects the first occurrence of `pattern` in the inspector message, like dragging over it. */
async function selectInMessage(page: Page, pattern: string): Promise<void> {
  await page.evaluate((src) => {
    const el = document.querySelector('debrief-inspector .message')!;
    const node = el.firstChild as Text;
    const m = new RegExp(src).exec(node.data)!;
    const range = document.createRange();
    range.setStart(node, m.index);
    range.setEnd(node, m.index + m[0].length);
    const sel = document.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  }, pattern);
}

async function selectRowWithMessage(page: Page, text: string): Promise<void> {
  await page.getByTestId('query-field').fill(text);
  const row = page.locator('debrief-events-grid [role=row].row').first();
  await expect(row).toContainText(text);
  await row.click();
  await expect(page.locator('debrief-inspector .message')).toContainText(text);
}

test.describe('context actions from data', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('right-click a row, drop events like it, and the total shrinks', async ({ page }, testInfo) => {
    const total = page.locator('debrief-events-grid .bar .total');
    await expect(total).toHaveText(/^\d[\d,]*$/);
    const before = parseCount(await total.innerText());

    await page.locator('debrief-events-grid [role=row].row').first().click({ button: 'right' });
    const menu = page.getByRole('menu', { name: 'Actions' });
    await expect(menu).toBeVisible();
    await expect(menu.getByRole('menuitem').first()).toBeFocused();
    await expectNoSeriousViolations(page, testInfo, 'row-context-menu');
    await menu.getByRole('menuitem', { name: /Drop events like this/ }).click();

    const dialog = page.getByRole('dialog', { name: 'Drop events like this' });
    await expect(dialog).toBeVisible();
    const status = dialog.getByRole('status');
    await expect(status).toContainText(/matches\s*[\d,]+\s*events/);
    const removed = parseCount((await status.innerText()).replace(/.*matches\s*/s, ''));
    expect(removed).toBeGreaterThan(0);
    await expectNoSeriousViolations(page, testInfo, 'drop-dialog');
    await dialog.getByRole('button', { name: 'Add drop step' }).click();
    await expect(dialog).toHaveCount(0);
    await expect.poll(async () => parseCount(await total.innerText())).toBe(before - removed);
  });

  test('the menu is keyboard accessible and Escape returns focus', async ({ page }) => {
    const row = page.locator('debrief-events-grid [role=row].row').first();
    await row.click();
    await page.getByRole('grid', { name: 'Events' }).focus();
    await page.keyboard.press('Shift+F10');
    const menu = page.getByRole('menu', { name: 'Actions' });
    await expect(menu).toBeVisible();
    await page.keyboard.press('ArrowDown');
    await expect(menu.getByRole('menuitem').nth(1)).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await expect(page.getByRole('grid', { name: 'Events' })).toBeFocused();
  });

  test('inspector property menu filters by the value', async ({ page }) => {
    await page.locator('debrief-events-grid [role=row].row').first().click();
    const prop = page.locator('debrief-inspector .prop').filter({ has: page.locator('.k', { hasText: /^MachineName$/ }) });
    await prop.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Only this MachineName' }).click();
    await expect(page.locator('debrief-filter-bar .chip').filter({ hasText: 'MachineName' })).toBeVisible();
  });

  test('mask dialog previews the replacement and saves', async ({ page }, testInfo) => {
    await selectRowWithMessage(page, 'Slow request');
    await selectInMessage(page, '\\d+ ms');
    await page.getByRole('button', { name: 'More actions…' }).click();
    await page.getByRole('menuitem', { name: 'Mask in message…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Mask in message' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId('mask-after')).toContainText('***');
    await expect(dialog.getByTestId('mask-before')).not.toContainText('***');
    await expectNoSeriousViolations(page, testInfo, 'mask-dialog');
    await dialog.getByRole('button', { name: 'Add mask step' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('status').filter({ hasText: 'mask step' })).toBeVisible();
    await page.getByTestId('query-field').fill('Slow request');
    await expect(page.locator('debrief-events-grid [role=row].row').first()).toContainText('***');
  });

  test('extract dialog proposes a regex, previews matches and adds a field', async ({ page }, testInfo) => {
    await selectRowWithMessage(page, 'Slow request');
    await selectInMessage(page, '\\d+(?= ms)');
    await page.getByRole('button', { name: 'More actions…' }).click();
    await page.getByRole('menuitem', { name: 'Extract value…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Extract value' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId('extract-preview')).toContainText(/\d+ of \d+ candidate events match/);
    await expectNoSeriousViolations(page, testInfo, 'extract-dialog');
    await dialog.getByRole('button', { name: 'Add extractor' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('status').filter({ hasText: 'field' })).toBeVisible();
  });

  test('mark dialog adds a marker rule', async ({ page }, testInfo) => {
    await page.locator('debrief-events-grid [role=row].row').first().click();
    await page.getByRole('button', { name: 'More actions…' }).click();
    await page.getByRole('menuitem', { name: 'Mark events like this…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Mark events like this' });
    await expect(dialog.getByRole('status')).toContainText(/marks\s*[\d,]+\s*events/);
    await expectNoSeriousViolations(page, testInfo, 'mark-dialog');
    await dialog.getByLabel('Label').fill('My marker');
    await dialog.getByRole('button', { name: 'Add marker' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('status').filter({ hasText: 'My marker' })).toBeVisible();
  });

  test('fields panel charts a numeric field in Metrics', async ({ page }) => {
    await expandFields(page);
    await page.locator('debrief-sidebar .field').filter({ hasText: 'ElapsedMs' }).click();
    await page.getByRole('button', { name: 'Chart this field' }).click();
    await expect(page.getByRole('tab', { name: 'Metrics', selected: true })).toBeVisible();
    await expect(page.locator('debrief-chart-card').filter({ hasText: 'avg(ElapsedMs)' })).toBeVisible();
  });
});
