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
    await expect(page.locator('debrief-welcome')).toHaveCount(0);

    // Opening a folder remembers it.
    await page.getByRole('button', { name: 'Open folder' }).click();
    await expect(page.locator('debrief-sidebar')).toContainText('extra-folder', { timeout: 10_000 });

    await closeWorkspace(page);
    const welcome = page.locator('debrief-welcome');
    await expect(welcome).toBeVisible();
    await expect(welcome.getByRole('heading', { name: 'Welcome to Debrief' })).toBeVisible();
    await expect(welcome.getByRole('list', { name: 'How Debrief works' }).getByRole('listitem')).toHaveCount(4);
    await expectNoSeriousViolations(page, testInfo, 'welcome');

    await welcome.getByRole('button', { name: 'extra-folder' }).click();
    await expect(welcome).toHaveCount(0);
    await expect(page.locator('debrief-sidebar .src:not(.dim)').first()).toBeVisible();
  });

  // Regression: with no files open, the Jobs and Notifications views opened behind the welcome screen.
  test('Jobs and Notifications open over an empty workspace', async ({ page }) => {
    await openApp(page);
    await closeWorkspace(page);
    const welcome = page.locator('debrief-welcome');
    await expect(welcome).toBeVisible();
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('View: Jobs');
    await page.keyboard.press('Enter');
    await expect(welcome).toHaveCount(0);
    await expect(page.locator('debrief-jobs-panel')).toBeVisible();
    await page.getByTestId('status-notifications').click();
    await expect(page.locator('debrief-notifications-panel')).toBeVisible();
  });

  test('"Show welcome" is a command and can be dismissed', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('welcome');
    await page.keyboard.press('Enter');
    await expect(page.locator('debrief-welcome')).toBeVisible();
    await page.getByRole('button', { name: 'Back to workspace' }).click();
    await expect(page.locator('debrief-welcome')).toHaveCount(0);
  });

  test('covers the dock completely: no sash shows through or can be grabbed', async ({ page }) => {
    await openApp(page);
    const sashes = page.locator('debrief-dock-host .dv-sash');
    await expect.poll(() => sashes.count()).toBeGreaterThan(0);
    const points = await sashes.evaluateAll((els) =>
      els
        .map((el) => el.getBoundingClientRect())
        .filter((r) => r.width > 0 && r.height > 0)
        .map((r) => ({ x: r.left + r.width / 2, y: r.top + r.height / 2 })),
    );
    expect(points.length).toBeGreaterThan(0);
    await closeWorkspace(page);
    await expect(page.locator('debrief-welcome')).toBeVisible();
    // At every sash position the topmost element belongs to the welcome screen.
    const covered = await page.evaluate(
      (pts) => pts.map((p) => !!document.elementFromPoint(p.x, p.y)?.closest('debrief-welcome')),
      points.filter((p) => p.y > 0),
    );
    expect(covered.every(Boolean)).toBe(true);
  });
});
