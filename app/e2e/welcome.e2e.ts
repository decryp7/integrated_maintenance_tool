import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

async function closeWorkspace(page: import('@playwright/test').Page): Promise<void> {
  await chooseMenu(page, 'File', 'Close workspace');
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

  // Regression: with no files open, the Jobs and Notifications views opened behind the welcome screen.
  test('Jobs and Notifications open over an empty workspace', async ({ page }) => {
    await openApp(page);
    await closeWorkspace(page);
    const welcome = page.locator('imt-welcome');
    await expect(welcome).toBeVisible();
    await page.locator('imt-activity-bar').getByRole('button', { name: /^Jobs/ }).click();
    await expect(welcome).toHaveCount(0);
    await expect(page.getByRole('tab', { name: /Jobs/ })).toBeVisible();
    await page.locator('imt-activity-bar').getByRole('button', { name: /^Notifications/ }).click();
    await expect(page.getByRole('tab', { name: /Notifications/ })).toBeVisible();
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

  test('covers the dock completely: no sash shows through or can be grabbed', async ({ page }) => {
    await openApp(page);
    const sashes = page.locator('imt-dock-host .dv-sash');
    await expect.poll(() => sashes.count()).toBeGreaterThan(0);
    const points = await sashes.evaluateAll((els) =>
      els
        .map((el) => el.getBoundingClientRect())
        .filter((r) => r.width > 0 && r.height > 0)
        .map((r) => ({ x: r.left + r.width / 2, y: r.top + r.height / 2 })),
    );
    expect(points.length).toBeGreaterThan(0);
    await closeWorkspace(page);
    await expect(page.locator('imt-welcome')).toBeVisible();
    // At every sash position the topmost element belongs to the welcome screen.
    const covered = await page.evaluate(
      (pts) => pts.map((p) => !!document.elementFromPoint(p.x, p.y)?.closest('imt-welcome')),
      points.filter((p) => p.y > 0),
    );
    expect(covered.every(Boolean)).toBe(true);
  });
});
