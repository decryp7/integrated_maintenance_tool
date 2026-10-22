import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

// What's new: shown once after an update (never on a first run), and always under Help.

const SEEN = 'debrief.whatsnew.seen';

test.describe("What's new", () => {
  test('opens once after an update, then stays closed', async ({ page }, testInfo) => {
    await page.addInitScript((key) => {
      if (!sessionStorage.getItem('seeded')) {
        sessionStorage.setItem('seeded', '1');
        localStorage.setItem(key, '0.0.1');
      }
    }, SEEN);
    await openApp(page);
    const dialog = page.getByRole('dialog', { name: "What's new" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId('whats-new-current')).toContainText('Version 0.1.0');
    await expect(dialog.getByTestId('whats-new-current').locator('li').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'whats-new', 'debrief-whats-new');
    await dialog.getByTestId('whats-new-close').click();
    await expect(dialog).toHaveCount(0);

    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(dialog).toHaveCount(0);
    expect(await page.evaluate((key) => localStorage.getItem(key), SEEN)).toBe('0.1.0');
  });

  test('a first run only records the version, and Help › What\'s new opens it on demand', async ({ page }) => {
    await openApp(page);
    await expect(page.getByRole('dialog', { name: "What's new" })).toHaveCount(0);
    await expect.poll(() => page.evaluate((key) => localStorage.getItem(key), SEEN)).toBe('0.1.0');
    await chooseMenu(page, 'Help', "What's new");
    await expect(page.getByRole('dialog', { name: "What's new" })).toContainText('Version 0.1.0');
  });
});
