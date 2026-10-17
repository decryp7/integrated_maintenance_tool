import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

async function closeWorkspace(page: import('@playwright/test').Page): Promise<void> {
  await page.getByRole('button', { name: 'File', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Close workspace' }).click();
}

test.describe('welcome', () => {
  test('appears for an empty workspace, is accessible and reopens a recent folder', async ({ page }, testInfo) => {
    await openApp(page);
    await expect(page.locator('imt-welcome')).toHaveCount(0);

    // Opening a folder remembers it.
    await page.getByRole('button', { name: 'Open folder' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('extra-folder', { timeout: 10_000 });

    await closeWorkspace(page);
    const welcome = page.locator('imt-welcome');
    await expect(welcome).toBeVisible();
    await expect(welcome.getByRole('heading', { name: 'Welcome to IMT' })).toBeVisible();
    await expect(welcome.getByRole('list', { name: 'How IMT works' }).getByRole('listitem')).toHaveCount(4);
    await expectNoSeriousViolations(page, testInfo, 'welcome');

    await welcome.getByRole('button', { name: 'extra-folder' }).click();
    await expect(welcome).toHaveCount(0);
    await expect(page.locator('imt-sidebar .src:not(.dim)').first()).toBeVisible();
  });

  test('"Show welcome" is a command and can be dismissed', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('welcome');
    await page.keyboard.press('Enter');
    await expect(page.locator('imt-welcome')).toBeVisible();
    await page.getByRole('button', { name: 'Back to workspace' }).click();
    await expect(page.locator('imt-welcome')).toHaveCount(0);
  });
});
