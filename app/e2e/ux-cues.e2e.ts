import { expect, test } from '@playwright/test';
import { openApp } from './app';

// Usability review 2026-10-23 (U11): a level or a lane is never shown by colour alone. In forced colours (Windows
// High Contrast) our colours are replaced, so the dots, the lane bars and the level codes must survive.

test.describe('forced colours', () => {
  test.use({ colorScheme: 'light' });

  test('level dots, level codes and lane bars stay visible', async ({ page }) => {
    await page.emulateMedia({ forcedColors: 'active' });
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
    const info = await page.evaluate(() => {
      const canvas = getComputedStyle(document.body).backgroundColor;
      const dot = (sel: string) => {
        const e = document.querySelector(sel) as HTMLElement | null;
        if (!e) return null;
        const c = getComputedStyle(e);
        return { bg: c.backgroundColor, shadow: c.boxShadow, w: e.getBoundingClientRect().width };
      };
      const code = document.querySelector('debrief-events-grid .lvl .sr') as HTMLElement | null;
      const bar = document.querySelector('debrief-timeline-lanes .lane-label');
      return {
        canvas,
        gridDot: dot('debrief-events-grid .lvl[data-level="Information"] .dot'),
        chipDot: dot('debrief-filter-bar .lvl[data-level="Error"] .dot'),
        codeWidth: code ? Math.round(code.getBoundingClientRect().width) : null,
        codeText: code?.textContent ?? '',
        laneBar: bar ? getComputedStyle(bar, '::before').backgroundColor : null,
      };
    });
    expect(info.gridDot?.bg).not.toBe(info.canvas);
    expect(info.chipDot?.bg).not.toBe(info.canvas);
    expect(info.chipDot?.w).toBeGreaterThan(0);
    // The quiet levels' codes (INF, DBG) are text on every row, not only for the notable ones.
    expect(info.codeWidth).toBeGreaterThan(8);
    expect(info.codeText).toMatch(/^[A-Z]{3}$/);
    expect(info.laneBar).not.toBe(info.canvas);
  });
});

test('levels have different shapes, not only colours', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await openApp(page);
  const clip = (level: string) => page.locator(`debrief-filter-bar .lvl[data-level="${level}"] .dot`).evaluate((e) => getComputedStyle(e).clipPath + '|' + getComputedStyle(e).borderRadius);
  const shapes = new Set([await clip('Error'), await clip('Warning'), await clip('Information'), await clip('Fatal')]);
  expect(shapes.size).toBe(4);
});
