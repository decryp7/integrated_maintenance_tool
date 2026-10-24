import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';
import { CHANGELOG } from '../src/app/whats-new/changelog.generated';

// What's new is a document of the dock: opened once in the background after an update (never on a first run, never
// again for the same version) with one calm notification that focuses it, and always under Help › What's new.

// The mock reports the newest changelog entry as the running version.
const APP_VERSION = CHANGELOG[0]?.version ?? '';
const SEEN = 'debrief.whatsnew.seen';

const tab = (page: Page) => page.getByRole('tab', { name: "What's new", exact: true });
const view = (page: Page) => page.locator('debrief-whats-new');
const notices = (page: Page) => page.locator('debrief-notifications-panel');

/** Starts as if the previous run was an older version (only on the first load of the test, not on a reload). */
async function seedUpdate(page: Page): Promise<void> {
  await page.addInitScript((key) => {
    if (!sessionStorage.getItem('seeded')) {
      sessionStorage.setItem('seeded', '1');
      localStorage.setItem(key, '0.0.1');
    }
  }, SEEN);
}

test.describe("What's new", () => {
  test('after an update it opens once as a background document with a notification that focuses it', async ({ page }, testInfo) => {
    await seedUpdate(page);
    await openApp(page);
    await expect(tab(page)).toHaveCount(1);
    // In the background: it does not take the front, and no dialog window opens.
    await expect(tab(page)).toHaveAttribute('aria-selected', 'false');
    await expect(page.getByRole('tab', { name: 'Timeline', exact: true })).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByRole('dialog', { name: "What's new" })).toHaveCount(0);
    expect(await page.evaluate(() => !!document.activeElement?.closest('debrief-whats-new'))).toBe(false);
    await expect.poll(() => page.evaluate((key) => localStorage.getItem(key), SEEN)).toBe(APP_VERSION);

    await page.getByTestId('status-notifications').click();
    await expect(notices(page)).toContainText(`Updated to ${APP_VERSION}`);
    await notices(page).getByRole('button', { name: "See what's new" }).click();
    await expect(tab(page)).toHaveAttribute('aria-selected', 'true');
    await expect(view(page).getByTestId('whats-new-current')).toContainText(`Version ${APP_VERSION}`);
    await expect(view(page).getByTestId('whats-new-current').locator('li').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'whats-new', 'debrief-whats-new');

    // Closed like any document; a later start of the same version does not bring it back.
    await tab(page).locator('.debrief-tab-close').click();
    await expect(tab(page)).toHaveCount(0);
    // The layout is saved after a short debounce: reload once it no longer holds the view.
    await expect.poll(() => page.evaluate(() => localStorage.getItem('debrief.layout') ?? '')).not.toContain('"whats-new"');
    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(tab(page)).toHaveCount(0);
    await page.getByTestId('status-notifications').click();
    await expect(notices(page)).toBeVisible();
    await expect(notices(page)).not.toContainText(`Updated to ${APP_VERSION}`);
  });

  test("a first run only records the version, and Help › What's new opens the document on demand", async ({ page }) => {
    await openApp(page);
    await expect.poll(() => page.evaluate((key) => localStorage.getItem(key), SEEN)).toBe(APP_VERSION);
    await expect(tab(page)).toHaveCount(0);
    await chooseMenu(page, 'Help', "What's new");
    await expect(tab(page)).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByRole('dialog', { name: "What's new" })).toHaveCount(0);
    await expect(view(page).getByTestId('whats-new-current')).toContainText(`Version ${APP_VERSION}`);
  });
});
