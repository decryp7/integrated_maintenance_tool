import { expect, Page, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';

const SHOTS = process.env['HOME_SHOTS'];
const WIDTHS = [320, 400, 480, 540, 560, 720, 1024];
const LONG_WINDOWS = 'C:\\ProgramData\\VeryLongVendorNameIncorporated\\ExtremelyDescriptiveProductSuiteName\\InstalledComponents\\CompanyA.Graphic.Builder.Service\\Diagnostics\\Output\\CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.dmp';

async function openHome(page: Page): Promise<void> {
  await page.goto('/?mockLongNames=1');
  await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();
  await expect(page.locator('debrief-home-view')).toBeVisible({ timeout: 10_000 });
  await expect(page.locator('debrief-home-view [data-testid=home-attention]')).toBeVisible();
  await expect(page.locator('debrief-home-view .finding').first()).toBeVisible();
}

/** Makes the Home pane exactly `width` px wide, as a narrow docked panel or floating window would. */
async function squeeze(page: Page, width: number): Promise<void> {
  await page.evaluate((w) => {
    const host = document.querySelector('debrief-home-view') as HTMLElement;
    host.style.cssText += `;position:fixed;top:0;left:0;z-index:9999;width:${w}px;height:900px;flex:none;`;
  }, width);
  await page.waitForTimeout(250);
}

/** Elements of Home whose content is wider than their box (the horizontal overflow the user saw). */
async function overflowing(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const host = document.querySelector('debrief-home-view') as HTMLElement;
    const scroll = host.querySelector('.scroll') as HTMLElement;
    const bad: string[] = [];
    if (scroll.scrollWidth > scroll.clientWidth) bad.push(`.scroll ${scroll.scrollWidth}>${scroll.clientWidth}`);
    const box = host.getBoundingClientRect();
    for (const el of host.querySelectorAll<HTMLElement>('section, .callout, .finding, .tiles > div, .actions, .bar, .findings, .text')) {
      const r = el.getBoundingClientRect();
      if (r.width === 0) continue;
      const label = `${el.tagName.toLowerCase()}.${el.className}`;
      if (el.scrollWidth > el.clientWidth + 1) bad.push(`${label} scroll ${el.scrollWidth}>${el.clientWidth}`);
      if (r.right > box.right + 1 || r.left < box.left - 1) bad.push(`${label} outside host (${Math.round(r.left)}..${Math.round(r.right)} of ${Math.round(box.right)})`);
    }
    // Buttons must stay inside the pane.
    for (const b of host.querySelectorAll<HTMLElement>('.finding button')) {
      const r = b.getBoundingClientRect();
      if (r.right > box.right + 1) bad.push(`button "${b.textContent?.trim()}" clipped at ${Math.round(r.right)}>${Math.round(box.right)}`);
    }
    return bad;
  });
}

test.describe('home in a narrow pane with very long paths and names', () => {
  for (const width of WIDTHS) {
    test(`nothing overflows at ${width} px`, async ({ page }, info) => {
      await openHome(page);
      await squeeze(page, width);
      if (SHOTS) await page.screenshot({ path: `${SHOTS}/${info.project.name}-${width}.png`, clip: { x: 0, y: 0, width, height: 900 } });
      if (SHOTS) {
        await page.evaluate(() => document.querySelector('debrief-home-view .scroll')?.scrollTo(0, 99999));
        await page.screenshot({ path: `${SHOTS}/${info.project.name}-${width}-bottom.png`, clip: { x: 0, y: 0, width, height: 900 } });
      }
      if (width <= 560 || process.env['HOME_ALL']) expect(await overflowing(page)).toEqual([]);
    });
  }

  test('a long path in Needs attention is shortened in the middle, keeps its file name, and its full value is in the tooltip', async ({ page }) => {
    await openHome(page);
    await squeeze(page, 320);
    const row = page.locator('[data-testid=home-attention] .callout').filter({ hasText: 'Diagnostics.2026-10-23.dmp' }).first();
    await expect(row).toBeVisible();
    const name = row.getByTestId('path-text').first();
    await expect(name).toHaveAttribute('title', /ProgramData.*Diagnostics\.2026-10-23\.dmp/);
    expect(await name.getAttribute('title')).toBe(LONG_WINDOWS);
    // The start is cut with an ellipsis (by layout), the end of the file name stays visible.
    const head = name.locator('.head');
    expect(await head.evaluate((e) => e.scrollWidth > e.clientWidth && getComputedStyle(e).textOverflow === 'ellipsis')).toBe(true);
    const tail = name.locator('.tail');
    await expect(tail).toHaveText(/\.dmp$/);
    expect(await tail.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(true);
    await expect(row.getByRole('button', { name: /Copy/ })).toBeVisible();
  });
});
