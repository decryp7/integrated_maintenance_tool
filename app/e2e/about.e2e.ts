import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test.describe('About IMT', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('opens from the palette, shows version and licence, searches the notices', async ({ page }, testInfo) => {
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await input.fill('About IMT');
    await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText('About IMT');
    await page.keyboard.press('Enter');

    const dialog = page.getByRole('dialog', { name: 'About IMT' });
    await expect(dialog).toBeVisible();
    await expect(dialog.locator('.version')).toHaveText(/^\d+\.\d+\.\d+/);
    await expect(dialog).toContainText('MIT OR Apache-2.0');

    // The notices load from a lazy chunk (no fetch, so the production CSP is fine) and are virtualised.
    const lines = dialog.locator('.line');
    await expect(lines.first()).toHaveText('# Third-party notices');
    expect(await lines.count()).toBeLessThan(80);

    const search = dialog.getByRole('searchbox', { name: 'Search notices' });
    await search.fill('serde_json');
    await expect(lines.first()).toContainText('serde_json');
    await expect(dialog.locator('.count')).toContainText(/^\d+ of \d+ lines$/);
    for (const text of await lines.allTextContents()) expect(text.toLowerCase()).toContain('serde_json');

    await expectNoSeriousViolations(page, testInfo, 'about');

    await dialog.getByRole('button', { name: 'View LICENSE-APACHE' }).click();
    await expect(dialog.getByRole('region', { name: 'LICENSE-APACHE text' })).toBeVisible();
    await search.fill('');
    await expect(lines.filter({ hasText: 'Apache License' }).first()).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
  });

  test('Help menu opens it too', async ({ page }) => {
    await chooseMenu(page, 'Help', 'About IMT');
    await expect(page.getByRole('dialog', { name: 'About IMT' })).toBeVisible();
  });
});
