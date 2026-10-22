import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// High contrast theme (WCAG AAA tokens) and "Follow system": the system's contrast preference picks it by default.

const html = (page: Page) => page.locator('html');

test.describe('high contrast', () => {
  test('is chosen automatically when the system asks for more contrast, and a chosen theme wins', async ({ page }) => {
    await page.emulateMedia({ contrast: 'more', colorScheme: 'dark' });
    await openApp(page);
    await expect(html(page)).toHaveAttribute('data-theme', 'contrast');
    await page.getByRole('button', { name: 'Settings' }).click();
    const dialog = page.getByRole('dialog', { name: 'Settings' });
    await dialog.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Appearance' }).click();
    await expect(dialog.locator('[data-theme-card=system] input')).toBeChecked();
    await dialog.locator('[data-theme-card=calm]').click();
    await expect(html(page)).toHaveAttribute('data-theme', 'calm');
    await dialog.locator('[data-theme-card=system]').click();
    await expect(html(page)).toHaveAttribute('data-theme', 'contrast');
  });

  test('follows the system live and uses the default theme without a preference', async ({ page }) => {
    await page.emulateMedia({ contrast: 'no-preference' });
    await openApp(page);
    await expect(html(page)).toHaveAttribute('data-theme', 'calm');
    await page.emulateMedia({ contrast: 'more' });
    await expect(html(page)).toHaveAttribute('data-theme', 'contrast');
  });

  for (const scheme of ['light', 'dark'] as const) {
    test(`${scheme}: the high contrast main window has strong outlines and passes axe`, async ({ page }, testInfo) => {
      await page.addInitScript((s) => localStorage.setItem('imt.prefs.v1', JSON.stringify({ theme: 'contrast', scheme: s })), scheme);
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await openApp(page);
      await expect(html(page)).toHaveAttribute('data-theme', 'contrast');
      expect(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--text').trim())).toBe(scheme === 'dark' ? '#ffffff' : '#000000');
      await page.locator('imt-events-grid [role=row].row').first().click();
      await expectNoSeriousViolations(page, testInfo, `high-contrast-${scheme}`);
      await page.screenshot({ path: testInfo.outputPath(`high-contrast-${scheme}.png`) });
    });
  }
});
