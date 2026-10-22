import { expect, test } from '@playwright/test';
import { openAppWith, openView } from './tools-support';

test.describe('saved queries and highlight rules', () => {
  test('saved queries re-apply the search', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Saved queries');
    const search = page.getByTestId('query-field');
    await search.fill('timed out');
    const panel = page.locator('imt-saved-queries-view');
    await panel.getByRole('textbox', { name: 'Saved query name' }).fill('Timeouts');
    await panel.getByRole('button', { name: 'Save current query' }).click();
    await search.fill('');
    await panel.getByRole('button', { name: 'Apply query Timeouts' }).click();
    await expect(search).toHaveValue('timed out');
  });

  test('highlight rules can be added and removed', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Saved queries');
    const panel = page.locator('imt-saved-queries-view');
    await panel.getByLabel('Highlight field').fill('@level');
    await panel.getByLabel('Highlight condition').selectOption('eq');
    await panel.getByRole('textbox', { name: 'Highlight value' }).fill('Error');
    await panel.getByRole('radio', { name: 'Colour #e9ae3f' }).click();
    await panel.getByRole('button', { name: 'Add rule' }).click();
    const rules = panel.getByRole('list', { name: 'Highlight rules' });
    await expect(rules).toContainText('@level');
    await expect(rules).toContainText('Error');

    // The grid paints matching rows with a left colour bar (not only a tint), and only those rows.
    await page.getByTestId('query-field').fill('failed');
    const painted = page.locator('imt-events-grid [role=row].row.hl');
    await expect(painted.first()).toBeVisible();
    await expect(painted.first().locator('.lvl')).toContainText('ERR');
    const bar = await painted.first().evaluate((el) => {
      const s = getComputedStyle(el, '::before');
      return { width: s.width, color: s.backgroundColor };
    });
    expect(bar).toEqual({ width: '4px', color: 'rgb(233, 174, 63)' });
    await expect(page.locator('imt-events-grid [role=row].row:not(.hl) .lvl', { hasText: 'ERR' })).toHaveCount(0);

    await rules.getByRole('button', { name: 'Delete rule @level' }).click();
    await expect(rules).toHaveCount(0);
    await expect(painted).toHaveCount(0);
  });
});
