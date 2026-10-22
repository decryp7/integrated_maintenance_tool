import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';

// Search everything (Mod+Shift+F) against the mock backend: the palette lists results by kind with the words marked,
// and choosing one goes where it points.

test.describe('search everything', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('Mod+Shift+F finds a source and choosing it filters the grid; Escape closes the palette', async ({ page }) => {
    const total = await initialTotal(page);
    await page.locator('imt-events-grid').first().click();
    await page.keyboard.press('ControlOrMeta+Shift+F');
    const dialog = page.getByRole('dialog', { name: 'Search everything' });
    await expect(dialog).toBeVisible();
    const input = dialog.getByRole('combobox', { name: 'Search everything' });
    await expect(input).toBeFocused();

    await input.fill('worker01');
    const sources = dialog.getByTestId('search-group-sources');
    await expect(sources.getByRole('option')).toHaveCount(1);
    await expect(sources.locator('mark')).toHaveText('worker01');

    await sources.getByRole('option').click();
    await expect(dialog).toBeHidden();
    await expect(gridTotal(page)).not.toHaveText(total);

    // Events by words, then Escape: closed without going anywhere.
    await page.keyboard.press('ControlOrMeta+Shift+F');
    await input.fill('payment gateway');
    await expect(dialog.getByTestId('search-group-events').getByRole('option').first()).toBeVisible();
    await expect(dialog.getByTestId('search-group-events').locator('mark').first()).toHaveText(/payment/i);
    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();

    await page.keyboard.press('ControlOrMeta+Shift+F');
    await input.fill('zzqqxx');
    await expect(dialog.getByTestId('search-everything-empty')).toContainText('zzqqxx');
  });
});
