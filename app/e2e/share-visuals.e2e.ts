import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand } from './tools-support';

async function written(page: Page): Promise<{ path: string; kind: string; bytes: number }[]> {
  return page.evaluate(() => ((globalThis as Record<string, unknown>)['__imtMockShareWritten'] as { path: string; kind: string; bytes: number }[]) ?? []);
}

test.describe('chart and timeline images', () => {
  test('exports a chart as PNG and SVG through the save dialog', async ({ page }, testInfo) => {
    await openAppWith(page);
    await page.getByRole('tab', { name: 'Metrics' }).click();
    const card = page.locator('imt-chart-card').first();
    await expect(card.locator('canvas').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'chart-actions', 'imt-chart-card');

    await card.getByRole('button', { name: /^Export chart .* as PNG$/ }).click();
    await expect(page.getByRole('status').filter({ hasText: /Saved .*\.png/ })).toBeVisible();
    await card.getByRole('button', { name: /^Export chart .* as SVG$/ }).click();
    await expect(page.getByRole('status').filter({ hasText: /Saved .*\.svg/ })).toBeVisible();
    const saved = await written(page);
    expect(saved.find((w) => w.kind === 'png')?.path).toMatch(/\.png$/);
    expect(saved.find((w) => w.kind === 'svg')?.bytes).toBeGreaterThan(500);
  });

  test('exports the timeline as SVG and PNG', async ({ page }) => {
    await openAppWith(page);
    await runCommand(page, 'Export timeline as SVG');
    await expect(page.getByRole('status').filter({ hasText: 'Saved timeline.svg' })).toBeVisible();
    await runCommand(page, 'Export timeline as PNG');
    await expect(page.getByRole('status').filter({ hasText: 'Saved timeline.png' })).toBeVisible();
    const saved = await written(page);
    expect(saved.map((w) => w.path)).toEqual(expect.arrayContaining(['timeline.svg', 'timeline.png']));
  });
});
