import { expect, test } from '@playwright/test';
import { initialTotal, openApp } from './app';

test.describe('selection and inspector', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('clicking the fatal marker selects that event', async ({ page }) => {
    await page.getByRole('button', { name: /^Fatal at / }).first().click();
    const inspector = page.locator('debrief-inspector');
    await expect(inspector.locator('.badge-lvl')).toHaveText('Fatal');
    const selected = page.locator('debrief-events-grid .row.sel');
    await expect(selected).toBeVisible();
    await expect(selected.locator('.lvl')).toContainText('FTL');
    await expect(inspector.locator('.message')).not.toBeEmpty();
  });

  test('inspector + adds a filter pill and removing it restores the total', async ({ page }) => {
    const total = page.locator('debrief-events-grid .bar .total');
    const all = await initialTotal(page);
    await page.locator('debrief-events-grid [role=row].row').first().click();

    const inspector = page.locator('debrief-inspector');
    const prop = inspector.locator('.prop').filter({ has: page.locator('.k', { hasText: /^MachineName$/ }) });
    await prop.hover();
    await prop.getByRole('button', { name: /^Filter MachineName equals / }).click();

    const pill = page.locator('debrief-filter-bar .chip').filter({ hasText: 'MachineName' });
    await expect(pill).toBeVisible();
    await expect(total).not.toHaveText(all);

    await pill.getByRole('button', { name: 'Remove filter MachineName' }).click();
    await expect(pill).toHaveCount(0);
    await expect(total).toHaveText(all);
  });

  test('arrow keys move the grid selection', async ({ page }) => {
    const rows = page.locator('debrief-events-grid [role=row].row');
    await rows.first().click();
    const sel = page.locator('debrief-events-grid .row.sel');
    const id = page.locator('debrief-inspector .head .faint').first();
    await expect(sel).toHaveCount(1);
    /** The inspector's id once it shows a record other than `previous` (never a transient empty one). */
    const nextId = async (previous: string) => {
      let text = '';
      await expect.poll(async () => (text = (await id.innerText()).trim()) !== '' && text !== previous).toBe(true);
      return text;
    };
    const first = await nextId('');

    await page.getByRole('grid', { name: 'Events' }).focus();
    await page.keyboard.press('ArrowDown');
    const second = await nextId(first);
    await page.keyboard.press('ArrowDown');
    await nextId(second);

    await page.keyboard.press('ArrowUp');
    await expect(id).toHaveText(second);
    await page.keyboard.press('ArrowUp');
    await expect(id).toHaveText(first);
    await expect(sel).toHaveCount(1);
  });
});
