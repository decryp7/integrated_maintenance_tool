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
  });

  test('default (Investigate) perspective: the timeline with the registry viewer beside it over the events, the inspector auto-hidden; Ctrl+/ focuses the search field above them', async ({ page }) => {
    const names = async (selected?: boolean) => (await page.getByRole('tab', selected ? { selected } : {}).locator('.imt-tab-label').allInnerTexts()).map((t) => t.trim()).sort();
    // Each group shows one tab: Timeline, the viewer of the opened registry export beside it, and Events below.
    await expect.poll(() => names(true)).toEqual(['Events', 'Registry', 'Timeline']);
    expect(await names()).toEqual(['Events', 'Registry', 'Timeline']);
    const timeline = (await page.locator('.dv-groupview', { has: page.getByRole('tab', { name: 'Timeline', exact: true }) }).boundingBox())!;
    const registry = (await page.locator('.dv-groupview', { has: page.getByRole('tab', { name: 'Registry', exact: true }) }).boundingBox())!;
    const events = (await page.locator('.dv-groupview', { has: page.getByRole('tab', { name: 'Events', exact: true }) }).boundingBox())!;
    expect(registry.x).toBeGreaterThanOrEqual(timeline.x + timeline.width - 2);
    expect(events.y).toBeGreaterThanOrEqual(timeline.y + timeline.height - 2);
    // The inspector waits as an auto-hidden tool window on the right edge; Overview (off by default) and Metrics open on demand.
    await expect(page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' })).toBeVisible();
    await expect(page.locator('imt-inspector')).toHaveCount(0);
    await expect(page.locator('imt-home-view')).toHaveCount(0);
    // The one search field is part of the filter bar, above the dock, and no Query panel exists.
    await expect(page.getByRole('tab', { name: 'Query' })).toHaveCount(0);
    await page.getByRole('grid', { name: 'Events' }).focus();
    await page.keyboard.press('ControlOrMeta+/');
    await expect(page.getByTestId('query-field')).toBeFocused();
    const field = (await page.locator('imt-query-field').boundingBox())!;
    expect(field.y + field.height).toBeLessThanOrEqual(timeline.y + 2);
  });

  test('explorer lists the sources', async ({ page }) => {
    const sidebar = page.locator('imt-sidebar');
    await expect(sidebar.getByText('Explorer', { exact: true })).toBeVisible();
    await expect(sidebar.locator('.src[title*="web01/orders-20261014.clef"]')).toBeVisible();
    await expect(sidebar.locator('.src:not(.dim)')).toHaveCount(6);
    expect(await sidebar.locator('.src:not(.dim) [data-testid=format-badge]').evaluateAll((els) => els.map((e) => e.getAttribute('title')))).toEqual(['json', 'json', 'text', 'json', 'csv', 'registry']);
    await expect(sidebar.locator('[data-testid=folder-heading]').first()).toBeVisible();
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
