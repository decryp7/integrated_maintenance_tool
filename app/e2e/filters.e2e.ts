import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';

test.describe('filters', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('level chips change the grid total', async ({ page }) => {
    const levels = page.getByRole('group', { name: 'Levels' });
    const total = gridTotal(page);
    const all = await initialTotal(page);
    const chipCount = async (code: string) => parseCount(await levels.getByRole('button', { name: new RegExp(`^${code}`) }).locator('.faint').innerText());
    const errors = await chipCount('ERR');
    const fatals = await chipCount('FTL');
    expect(errors).toBeLessThan(1000); // chip counts are compact above 1k

    await levels.getByRole('button', { name: /^ERR/ }).click();
    await expect(levels.getByRole('button', { name: /^ERR/ })).toHaveAttribute('aria-pressed', 'true');
    await expect(total).toHaveText(String(errors));

    await levels.getByRole('button', { name: /^FTL/ }).click();
    await expect(total).toHaveText(String(errors + fatals));

    await levels.getByRole('button', { name: /^ERR/ }).click();
    await levels.getByRole('button', { name: /^FTL/ }).click();
    await expect(total).toHaveText(all);
  });

  test('text search narrows results and clearing restores them', async ({ page }) => {
    const total = gridTotal(page);
    const all = await initialTotal(page);
    const search = page.getByRole('textbox', { name: 'Search messages and exceptions' });
    await search.fill('timed out');
    // Wait for the narrowed total itself (not just "different": the bar can be between two states).
    await expect.poll(async () => {
      const n = parseCount(await total.innerText());
      return n > 0 && n < parseCount(all);
    }).toBe(true);
    await expect(page.locator('imt-status-bar')).toContainText('matching');
    await expect(page.locator('imt-events-grid [role=row].row').first()).toContainText(/time/i);

    await search.fill('');
    await expect(total).toHaveText(all);
  });

  test('search with no hits shows the empty state', async ({ page }) => {
    await page.getByRole('textbox', { name: 'Search messages and exceptions' }).fill('zzzz-no-such-text-zzzz');
    await expect(gridTotal(page)).toHaveText('0');
    await expect(page.getByText('No events match the current filters.')).toBeVisible();
  });
});
