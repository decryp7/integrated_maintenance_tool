import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { gridRows, openAppWith } from './mock-url';
import { expandFields } from './app';

test.describe('panel error boundaries', () => {
  test('a panel that throws shows an error card and leaves the rest of the app working', async ({ page }, testInfo) => {
    // Rows without `markers` make the events grid throw while rendering.
    await openAppWith(page, '?mockCorrupt=markers', { grid: false });
    const card = page.locator('.imt-panel-failed');
    await expect(card).toBeVisible();
    await expect(card).toContainText('This panel failed:');
    await expect(card.getByRole('button', { name: 'Reload panel' })).toBeVisible();
    await expect(card.getByRole('button', { name: 'Copy details' })).toBeVisible();
    // Other panels are untouched: the timeline renders and the layout is intact.
    await expect(page.locator('imt-timeline-panel .lane').first()).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Timeline' })).toBeVisible();
    await expect(page.locator('imt-status-bar')).toContainText('events');
    await expectNoSeriousViolations(page, testInfo, 'resilience-panel-failed');
  });

  test('Reload panel tries again, and Copy details reports the outcome', async ({ page }) => {
    await openAppWith(page, '?mockCorrupt=markers', { grid: false });
    const card = page.locator('.imt-panel-failed');
    await expect(card).toBeVisible();
    await card.getByRole('button', { name: 'Reload panel' }).click();
    // The data is still corrupt, so the panel fails again, with a fresh card.
    await expect(card).toBeVisible();
    await expect(card).toHaveCount(1);
    await card.getByRole('button', { name: 'Copy details' }).click();
    await expect(card.getByRole('button', { name: /Copied|Copy failed/ })).toBeVisible();
  });

  test('a failed page request shows an inline error with Retry', async ({ page }, testInfo) => {
    // The grid asks for its first page again when the workspace summary arrives, so two requests fail before one succeeds.
    await openAppWith(page, '?mockFail=queryPage:2');
    const err = page.locator('imt-events-grid .err-card');
    await expect(err).toContainText("Couldn't load events.");
    // The backend's own message is folded under Technical details.
    await expect(err.getByText('Simulated queryPage failure')).toBeHidden();
    await err.getByText('Technical details').click();
    await expect(err.getByText('Simulated queryPage failure')).toBeVisible();
    await expect(err).toContainText('Simulated queryPage failure');
    await expectNoSeriousViolations(page, testInfo, 'resilience-grid-error', 'imt-events-grid');
    await expect(async () => {
      if (await err.count()) await err.getByRole('button', { name: 'Retry' }).click();
      await expect(err).toHaveCount(0, { timeout: 1500 });
    }).toPass({ timeout: 15_000 });
    await expect(gridRows(page).first()).toBeVisible();
  });

  test('a failed facet request shows an inline error with Retry', async ({ page }) => {
    await openAppWith(page, '?mockFail=queryFacet:1');
    // Open the popover only once the workspace has settled: a query change while it is open reloads the facet,
    // and that newer (successful) request would supersede the failing one before its error is shown.
    await expect(gridRows(page).first()).toBeVisible();
    await expandFields(page);
    await page.locator('imt-sidebar .field').first().click();
    const pop = page.locator('imt-facet-popover');
    await expect(pop.getByRole('alert')).toContainText('Simulated queryFacet failure');
    await pop.getByRole('button', { name: 'Retry' }).click();
    await expect(pop.getByRole('alert')).toHaveCount(0);
    await expect(pop.locator('.value').first()).toBeVisible();
  });
});
