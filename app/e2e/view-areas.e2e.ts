import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';

// Views open in their home areas: analysis views share ONE group below the events grid, documents sit beside the
// timeline, and the timeline and the events are never covered.

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groupOf = (page: Page, name: string) => page.locator('.dv-groupview').filter({ has: tab(page, name) });

test.describe('view home areas', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
  });

  test('Anomalies, Patterns and Exceptions land in one group below the events grid; timeline and events stay visible', async ({ page }, testInfo) => {
    const timeline = page.locator('imt-timeline-panel');
    const grid = page.getByRole('grid', { name: 'Events' });
    const eventsBefore = (await groupOf(page, 'Events').boundingBox())!;
    await runCommand(page, 'Show anomalies');
    await runCommand(page, 'Show patterns');
    await runCommand(page, 'Show exceptions');

    const analysis = groupOf(page, 'Exceptions');
    for (const name of ['Anomalies', 'Patterns', 'Exceptions']) await expect(analysis.getByRole('tab', { name, exact: true })).toBeVisible();
    const events = (await groupOf(page, 'Events').boundingBox())!;
    const below = (await analysis.boundingBox())!;
    expect(below.y).toBeGreaterThanOrEqual(events.y + events.height - 2);
    // The events grid is still there (it only got shorter), and so is the timeline.
    expect(events.height).toBeLessThan(eventsBefore.height);
    expect(events.height).toBeGreaterThan(80);
    await expect(grid).toBeVisible();
    await expect(timeline).toBeVisible();
    await expect(timeline.locator('.lane').first()).toBeVisible();
    await expect(page.locator('imt-events-grid [role=row].row').first()).toBeVisible();
    // Exactly one group holds the analysis views.
    await expect(page.locator('.dv-groupview').filter({ has: tab(page, 'Anomalies') })).toHaveCount(1);
    await expect(page.locator('.dv-groupview').filter({ has: tab(page, 'Patterns') })).toHaveCount(1);
    await expectNoSeriousViolations(page, testInfo, 'view-areas-analysis');
  });

  test('a document (Metrics) opens beside the timeline, not over it or the events', async ({ page }) => {
    await page.getByRole('button', { name: 'Metrics', exact: true }).click();
    await expect(tab(page, 'Metrics')).toBeVisible();
    const metrics = (await groupOf(page, 'Metrics').boundingBox())!;
    const timeline = (await groupOf(page, 'Timeline').boundingBox())!;
    const events = (await groupOf(page, 'Events').boundingBox())!;
    expect(metrics.x).toBeGreaterThanOrEqual(timeline.x + timeline.width - 2);
    expect(metrics.y + metrics.height).toBeLessThanOrEqual(events.y + 2);
    await expect(tab(page, 'Timeline')).toHaveAttribute('aria-selected', 'true');
    await expect(tab(page, 'Events')).toHaveAttribute('aria-selected', 'true');
  });

  test('a view open behind another shows a dot on its activity button; the accent bar is for a view in front', async ({ page }, testInfo) => {
    const button = page.locator('imt-activity-bar [data-activity-id=metrics]');
    const dot = button.getByTestId('open-elsewhere');
    await expect(dot).toHaveCount(0);
    await button.click();
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'true');
    await expect(button).toHaveClass(/\bon\b/);
    await expect(dot).toHaveCount(0);
    // Bring the Registry viewer (the other tab of that group) to the front: Metrics is open, but behind.
    await tab(page, 'Registry').click();
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'false');
    await expect(dot).toHaveCount(1);
    await expect(button).not.toHaveClass(/\bon\b/);
    await expectNoSeriousViolations(page, testInfo, 'view-areas-open-elsewhere', 'imt-activity-bar');
    // Clicking the button brings it to the front: the dot goes, the accent comes.
    await button.click();
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'true');
    await expect(dot).toHaveCount(0);
    await expect(button).toHaveClass(/\bon\b/);
  });

  test('status tools (Jobs) slide out of the bottom edge and leave the timeline and events alone', async ({ page }) => {
    await runCommand(page, 'View: Jobs');
    await expect(page.locator('#imt-ah-flyout')).toBeVisible();
    await expect(page.locator('#imt-ah-flyout imt-jobs-panel')).toBeVisible();
    await expect(page.locator('imt-timeline-panel')).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(page.locator('.imt-ah-tab[data-ah-id="jobs"]')).toBeVisible();
    await expect(page.locator('.dv-groupview').filter({ has: tab(page, 'Jobs') })).toHaveCount(0);
  });
});
