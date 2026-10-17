import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand } from './tools-support';

/** What the mock share backend "wrote" (it records every report and image save). */
async function written(page: Page): Promise<{ path: string; kind: string; bytes: number }[]> {
  return page.evaluate(() => ((globalThis as Record<string, unknown>)['__imtMockShareWritten'] as { path: string; kind: string; bytes: number }[]) ?? []);
}

test.describe('incident report builder', () => {
  test('builds a report from a bookmark and a chart and exports it as HTML', async ({ page }, testInfo) => {
    await openAppWith(page);
    await page.locator('imt-events-grid [role=row].row').nth(2).click();
    await runCommand(page, 'Bookmark selected event');
    await runCommand(page, 'Add bookmarks to report');

    const view = page.locator('imt-report-view');
    await expect(view.getByRole('group', { name: /^Events block/ })).toBeVisible();

    await page.getByRole('tab', { name: 'Metrics' }).click();
    const card = page.locator('imt-chart-card').first();
    await expect(card.locator('canvas').first()).toBeVisible();
    await card.getByRole('button', { name: /^Add chart .* to report$/ }).click();
    await expect(page.getByRole('status').filter({ hasText: 'to the report' })).toBeVisible();

    await runCommand(page, 'Show report builder');
    const chart = view.getByRole('group', { name: /^Chart block/ });
    await expect(chart).toBeVisible();
    await expect(chart.locator('img')).toHaveAttribute('src', /^data:image\/svg\+xml/);
    await view.getByRole('textbox', { name: 'Report title' }).fill('Checkout outage');
    await view.getByRole('textbox', { name: 'Report title' }).press('Tab');

    // Keyboard reordering: the chart moves above the events.
    await chart.focus();
    await page.keyboard.press('Alt+ArrowUp');
    await expect(view.locator('imt-report-block').first()).toHaveAttribute('aria-label', /^Chart block/);
    await expect(view.locator('imt-report-block').first()).toBeFocused();
    await expectNoSeriousViolations(page, testInfo, 'report-builder', 'imt-report-view');

    await runCommand(page, 'Export report');
    const dialog = page.getByRole('dialog', { name: 'Export report' });
    await dialog.getByRole('checkbox', { name: "Redact with the profile's rules" }).check();
    await expect(dialog.getByRole('status', { name: 'Redaction preview' })).toContainText(/report text|referenced event/);
    await expectNoSeriousViolations(page, testInfo, 'report-export');
    await dialog.getByRole('button', { name: 'Export…' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('status').filter({ hasText: 'Exported the report to checkout-outage.html' })).toBeVisible();
    expect((await written(page)).some((w) => w.kind === 'html' && w.path === 'checkout-outage.html')).toBe(true);
  });

  test('adds text, sections and the timeline, and removes blocks', async ({ page }) => {
    await openAppWith(page);
    await runCommand(page, 'New incident report');
    const view = page.locator('imt-report-view');
    await view.getByRole('toolbar', { name: 'Add to report' }).getByRole('button', { name: 'Text' }).click();
    const text = view.getByRole('textbox', { name: /^Text of / });
    await text.fill('# Impact\nPayments **failed**');
    await text.press('Tab');
    await expect(view.locator('imt-markdown-view strong')).toHaveText('failed');
    await view.getByRole('button', { name: 'Add section' }).click();
    await view.getByRole('toolbar', { name: 'Add to report' }).getByRole('button', { name: 'Timeline' }).click();
    await expect(view.getByRole('group', { name: /^Timeline block/ }).locator('img')).toBeVisible();
    await view.getByRole('button', { name: /^Remove: Timeline block/ }).click();
    await expect(view.getByRole('group', { name: /^Timeline block/ })).toHaveCount(0);
  });
});
