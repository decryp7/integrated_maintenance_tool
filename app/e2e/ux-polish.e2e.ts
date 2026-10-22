import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, openMenuPath } from './menu-support';
import { openAppWith, openView } from './tools-support';

// The UX polish pass (audit items F5, F7, H2, J1, K1, K2, K5, I4).

test.describe('Open recent and the welcome page (F5, J1)', () => {
  test('File > Open recent lists what was opened, and the welcome page lists it with Open project', async ({ page }, testInfo) => {
    await openApp(page);
    await page.getByRole('button', { name: 'Open folder' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('extra-folder', { timeout: 10_000 });

    const recent = await openMenuPath(page, 'File', 'Open recent');
    await expect(recent.getByRole('menuitem', { name: 'extra-folder' })).toBeVisible();
    await expect(recent.getByRole('menuitem', { name: 'Clear recent' })).toBeVisible();
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');

    await chooseMenu(page, 'File', 'Close workspace');
    const welcome = page.locator('imt-welcome');
    await expect(welcome.getByTestId('welcome-recent')).toContainText('extra-folder');
    // J1: and when it was opened.
    await expect(welcome.getByTestId('recent-when').first()).not.toHaveText('');
    await expect(welcome.getByRole('button', { name: 'Open project…' })).toBeVisible();
    // The empty Explorer is not drawn next to the welcome page.
    await expect(page.locator('imt-sidebar')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'welcome-recent');

    await chooseMenu(page, 'File', 'Open recent', 'Clear recent');
    await expect(welcome.getByTestId('welcome-recent')).toHaveCount(0);
  });
});

test.describe('timeline legend (F7) and explorer (H2)', () => {
  test('the legend is one row until +N more, and the explorer rows keep their names under folder headings', async ({ page }) => {
    await openApp(page);
    const row = page.locator('imt-timeline-panel .legend-row');
    await expect(row).toBeVisible();
    const heights = await row.evaluate((el) => ({ row: el.getBoundingClientRect().height, line: el.querySelector('.lg')!.getBoundingClientRect().height }));
    // One line of 24 px hit areas, whatever the panel width: the full legend opens as a popover over the lanes.
    expect(heights.row).toBeLessThan(heights.line * 1.8);
    const headings = page.locator('imt-sidebar [data-testid=folder-heading]');
    await expect(headings.first()).toBeVisible();
    await expect(page.locator('imt-sidebar .src:not(.dim)').first()).toHaveAttribute('title', /Format: /);
  });
});

test.describe('inspector (K1), anomalies (K2)', () => {
  test('More actions holds Mask and Extract with a reason, and a property value can be copied', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']).catch(() => undefined);
    await openApp(page);
    await page.locator('imt-events-grid [role=row].row').first().click();
    await expect(page.getByRole('button', { name: 'Mask…' })).toHaveCount(0);
    await page.getByRole('button', { name: 'More actions…' }).click();
    const mask = page.getByRole('menuitem', { name: 'Mask in message…' });
    await expect(mask).toBeDisabled();
    await expect(page.getByRole('menu')).toContainText('Select text in the message first');
    await page.keyboard.press('Escape');
    await expect(page.locator('imt-inspector [data-testid=prop-copy]').first()).toBeAttached();
  });

  test('the Anomalies table has sortable headers', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Anomalies');
    const head = page.locator('imt-anomalies-view [data-testid=anomaly-head]');
    await expect(head.getByRole('button', { name: 'Sort by score' })).toBeVisible();
    // The list is grouped by kind, so the headers sort inside each group.
    await head.getByRole('button', { name: 'Sort by score' }).click();
    await expect(head.getByRole('button', { name: 'Sort by score' })).toHaveAttribute('aria-pressed', 'true');
  });
});

test.describe('Jobs filters (K5) and Plugins tabs (I4)', () => {
  test('Jobs offers All, Running and Failed and a Kind filter with counts', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Jobs');
    const jobs = page.locator('imt-jobs-panel');
    await expect(jobs.getByTestId('jobs-view')).toBeVisible();
    await expect(jobs.locator('.chip')).toHaveCount(0);
    await jobs.getByTestId('jobs-view-failed').click();
    await expect(jobs.getByTestId('jobs-view-failed')).toHaveAttribute('aria-pressed', 'true');
    await jobs.getByTestId('kind-filter').click();
    // A CDK listbox in an overlay: options by role, toggled with the keyboard, closed with Escape.
    const list = page.getByTestId('kind-filter-list');
    await expect(list).toContainText('AI');
    await expect(list.getByRole('listbox')).toHaveAttribute('aria-multiselectable', 'true');
    await list.getByRole('option').first().click();
    await expect(jobs.getByTestId('kind-filter')).toContainText('Kind: 1');
    await page.keyboard.press('Escape');
    await expect(list).toHaveCount(0);
  });

  test('Plugins is one view with Installed and Health tabs', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Plugins');
    await expect(page.getByTestId('plugins-tab-installed')).toHaveAttribute('aria-selected', 'true');
    await page.getByTestId('plugins-tab-health').click();
    await expect(page.locator('imt-plugin-health-view')).toBeVisible();
  });
});
