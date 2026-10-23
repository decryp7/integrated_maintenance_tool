import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';

/** Opens the Lens menu and picks an item by its text. */
async function lensMenu(page: Page, item: string | RegExp): Promise<void> {
  await page.getByTestId('lens-button').click();
  await page.getByRole('menuitem', { name: item, exact: typeof item === 'string' }).click();
}

test.describe('lenses and filter sets', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('switching a profile lens sets the levels and the grid follows', async ({ page }) => {
    const levels = page.getByRole('toolbar', { name: 'Levels' });
    const total = gridTotal(page);
    const all = await initialTotal(page);
    const chipCount = async (code: string) => parseCount(await levels.getByRole('button', { name: new RegExp(`^${code}`) }).locator('.faint').innerText());
    const errors = await chipCount('ERR');
    const fatals = await chipCount('FTL');

    await lensMenu(page, 'Errors only');
    await expect(levels.getByRole('button', { name: /^ERR/ })).toHaveAttribute('aria-pressed', 'true');
    await expect(levels.getByRole('button', { name: /^FTL/ })).toHaveAttribute('aria-pressed', 'true');
    await expect(total).toHaveText(String(errors + fatals));

    await lensMenu(page, 'Warnings and worse');
    await expect(levels.getByRole('button', { name: /^WRN/ })).toHaveAttribute('aria-pressed', 'true');
    // The lens also brings its extra column.
    await expect(page.locator('debrief-events-grid .hcell.extra', { hasText: 'SourceContext' })).toBeVisible();
    await expect(total).not.toHaveText(all);
  });

  test('a saved lens returns to the saved view', async ({ page }) => {
    const total = gridTotal(page);
    const all = await initialTotal(page);
    const levels = page.getByRole('toolbar', { name: 'Levels' });
    await levels.getByRole('button', { name: /^ERR/ }).click();
    const errorsOnly = await total.innerText();

    await lensMenu(page, /Save current as lens/);
    await page.getByTestId('lens-save-form').getByRole('textbox', { name: 'Name' }).fill('My errors');
    await page.getByRole('button', { name: 'Save lens' }).click();
    await levels.getByRole('button', { name: /^ERR/ }).click();
    await expect(total).toHaveText(all);

    await lensMenu(page, 'My errors');
    await expect(total).toHaveText(errorsOnly);
  });

  test('filter sets switch off and on without being deleted', async ({ page }) => {
    const total = gridTotal(page);
    const all = await initialTotal(page);
    await lensMenu(page, 'Errors only');
    await expect(total).not.toHaveText(all);
    const errorsOnly = await total.innerText();

    // Group a filter into a set: the chip appears switched on and the filter still applies.
    await page.getByRole('button', { name: '+ Add filter' }).click();
    await page.getByLabel('Field', { exact: true }).fill('SourceContext');
    await page.getByLabel('Value', { exact: true }).fill('zz-no-such-context');
    await page.getByRole('button', { name: 'Add', exact: true }).click();
    await expect(total).toHaveText('0');
    await lensMenu(page, 'Filters → set');

    const chip = page.getByTestId('filter-set').getByRole('button', { name: /^Set 1/ });
    await expect(chip).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('group', { name: 'Filters', exact: true })).toHaveCount(0);
    await expect(total).toHaveText('0');

    await chip.click();
    await expect(chip).toHaveAttribute('aria-pressed', 'false');
    await expect(total).toHaveText(errorsOnly);

    await page.keyboard.press('Alt+Digit1');
    await expect(chip).toHaveAttribute('aria-pressed', 'true');
    await expect(total).toHaveText('0');

    await page.getByRole('button', { name: 'Delete filter set Set 1' }).click();
    await expect(page.getByTestId('filter-set')).toHaveCount(0);
  });
});
