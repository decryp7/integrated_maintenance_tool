import { expect, test } from '@playwright/test';
import { openApp } from './app';

test.describe('app load', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('renders the shell', async ({ page }) => {
    await expect(page).toHaveTitle(/.+/);
    await expect(page.getByRole('button', { name: 'Explorer' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Timeline', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Metrics', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Profile editor' })).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Timeline', selected: true })).toBeVisible();
    // Timeline, Events and Inspector are the visible tab of their group; Metrics, Profile and Registry are stacked behind Timeline.
    await expect(page.getByRole('tab', { selected: true })).toHaveCount(3);
    await expect(page.getByRole('tab')).toHaveCount(6);
  });

  test('explorer lists the sources', async ({ page }) => {
    const sidebar = page.locator('imt-sidebar');
    await expect(sidebar.getByText('Explorer', { exact: true })).toBeVisible();
    await expect(sidebar.getByText('web01/orders-20261014.clef')).toBeVisible();
    await expect(sidebar.locator('.src:not(.dim)')).toHaveCount(6);
    await expect(sidebar.locator('.src:not(.dim) .badge')).toHaveText(['json', 'json', 'text', 'json', 'csv', 'registry']);
    await expect(sidebar.locator('.src.dim')).toHaveCount(1); // unrecognized files are listed dimmed
    await expect(sidebar.getByText('Fields', { exact: true })).toBeVisible();
  });

  test('status bar shows the event count', async ({ page }) => {
    const status = page.locator('imt-status-bar');
    await expect(status).toContainText('6 sources');
    await expect(status).toContainText(/\d[\d,]+ events/);
    await expect(status).toContainText(/\d[\d,]+ matching/);
    await expect(status).toContainText('7 instances');
  });

  test('timeline shows a lane per instance', async ({ page }) => {
    const lanes = page.locator('imt-timeline-panel .lane-label');
    await expect(lanes).toHaveCount(7);
    await expect(lanes.filter({ hasText: 'WEB02' })).toHaveCount(2);
    await expect(page.locator('imt-timeline-panel')).toContainText('7 instances');
  });
});
