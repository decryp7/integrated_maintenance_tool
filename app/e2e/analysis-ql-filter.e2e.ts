import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { openQuery } from './analysis-support';

test.describe('query language filters the workspace', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('running a filter query narrows the grid, clearing it restores the count', async ({ page }) => {
    const before = await initialTotal(page);
    const input = await openQuery(page);
    await input.click();
    await input.fill('level >= error');
    await input.press('ControlOrMeta+Enter');
    await expect(page.locator('imt-query-view .applied')).toContainText('level >= error');

    await page.getByRole('tab', { name: 'Events', exact: true }).click();
    await expect(gridTotal(page)).not.toHaveText(before);
    const filtered = parseCount(await gridTotal(page).innerText());
    expect(filtered).toBeGreaterThan(0);
    expect(filtered).toBeLessThan(parseCount(before));
    const rows = page.locator('imt-events-grid [role=row].row');
    await expect(rows.first()).toContainText(/ERR|FTL/);
    await expect(rows.filter({ hasText: /INF|WRN|DBG|VRB/ })).toHaveCount(0);

    await page.getByRole('tab', { name: 'Query', exact: true }).click();
    await page.locator('imt-query-view .applied').getByRole('button', { name: 'Clear' }).click();
    await page.getByRole('tab', { name: 'Events', exact: true }).click();
    await expect(gridTotal(page)).toHaveText(before);
  });
});
