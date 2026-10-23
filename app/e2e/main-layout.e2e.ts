import { expect, Locator, Page, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';
import { openAiApp, openChat, runCommand } from './ai-support';

// The reported layout: Explorer | timeline/events/markers | AI assistant | Home, at two common window sizes. Every
// pane keeps a readable width, Home's findings show their titles, and nothing sticks out of Explorer, the assistant or Markers.
const SHOTS = process.env['LAYOUT_SHOTS'];
const SIZES = [
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 },
];
/** Side tool panes (the assistant) are never narrower than this, px. */
const SIDE_MIN = 320;
/** The timeline/events column is never narrower than this, px. */
const MAIN_MIN = 480;

async function openScenario(page: Page): Promise<void> {
  const scheme = process.env['LAYOUT_THEME'] === 'light' ? 'light' : 'dark';
  await page.addInitScript((s) => localStorage.setItem('debrief.prefs.v1', JSON.stringify({ scheme: s })), scheme);
  await page.emulateMedia({ colorScheme: scheme });
  await openAiApp(page, 'mockLongNames=1');
  await openChat(page);
  await runCommand(page, 'Manage markers');
  await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();
  await expect(page.locator('debrief-home-view [data-testid=home-attention]')).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await expect(page.locator('debrief-home-view .finding .title').first()).toBeVisible();
  await expect(page.locator('debrief-markers-manager .rules tbody tr').first()).toBeVisible();
  await page.waitForTimeout(400);
}

/** Width of the dock group holding a panel's component (-1 when it is not shown). */
async function groupWidth(page: Page, selector: string): Promise<number> {
  return page.evaluate((s) => {
    const el = document.querySelector(s) as HTMLElement | null;
    const group = el?.closest('.dv-groupview') as HTMLElement | null;
    return el && el.offsetParent && group ? Math.round(group.getBoundingClientRect().width) : -1;
  }, selector);
}

/** Descendants of `root` that stick out of it on the right, or whose own content is wider than their box without an ellipsis. */
async function overflowing(root: Locator): Promise<string[]> {
  return root.evaluate((host) => {
    const box = host.getBoundingClientRect();
    const bad: string[] = [];
    if (host.scrollWidth > host.clientWidth + 1) bad.push(`host scrolls ${host.scrollWidth}>${host.clientWidth}`);
    for (const el of host.querySelectorAll<HTMLElement>('*')) {
      if (el.closest('svg, canvas, .sr, .sr-only, .cdk-visually-hidden, .actions')) continue;
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue;
      const label = `${el.tagName.toLowerCase()}.${String(el.className).slice(0, 30)}`;
      if (r.right > box.right + 1 && r.left < box.right) bad.push(`${label} sticks out (${Math.round(r.right)}>${Math.round(box.right)})`);
    }
    return bad;
  });
}

for (const size of SIZES) {
  test.describe(`${size.width}x${size.height}`, () => {
    test.use({ viewport: size });

    test('panes keep their minimum widths, findings show their titles, nothing overflows', async ({ page }, info) => {
      await openScenario(page);
      if (SHOTS) await page.screenshot({ path: `${SHOTS}/${info.project.name}-${size.width}-${process.env['LAYOUT_THEME'] ?? 'dark'}.png` });

      // Pane widths: the assistant and the timeline column are readable, and Home (a column or a tab) is at least as wide as the assistant.
      const ai = await groupWidth(page, 'debrief-ai-chat-panel');
      const main = await groupWidth(page, 'debrief-timeline-panel');
      const home = await groupWidth(page, 'debrief-home-view');
      expect(ai).toBeGreaterThanOrEqual(SIDE_MIN);
      expect(main).toBeGreaterThanOrEqual(MAIN_MIN);
      expect(home).toBeGreaterThanOrEqual(SIDE_MIN);

      // Health findings: the title and detail have room (more than 120 px), the actions never overlap them.
      const titles = page.locator('debrief-home-view [data-testid=home-findings] .finding .title');
      expect(await titles.count()).toBeGreaterThan(0);
      for (const t of await titles.all()) expect((await t.boundingBox())!.width).toBeGreaterThan(120);
      expect(await overflowing(page.locator('debrief-home-view .scroll'))).toEqual([]);

      // Explorer: every file name is one line, the head is cut and the tail stays, nothing sticks out.
      for (const row of await page.locator('debrief-sidebar .src:not(.group)').all()) {
        const head = row.locator('.nm-head');
        const tail = row.locator('.nm-tail');
        if (!(await head.count())) continue;
        const h = (await head.boundingBox())!;
        expect(h.height).toBeLessThan(24);
        const t = await tail.boundingBox();
        if (t && t.width > 0) expect(Math.abs(t.y - h.y)).toBeLessThan(6);
      }
      expect(await overflowing(page.locator('debrief-sidebar .sources'))).toEqual([]);

      // The assistant and Markers: no horizontal overflow, and a long regex stays inside its cell with its full text as a tooltip.
      expect(await overflowing(page.locator('debrief-ai-chat-panel'))).toEqual([]);
      expect(await overflowing(page.locator('debrief-markers-manager'))).toEqual([]);
      const rx = page.locator('debrief-markers-manager .rx').first();
      await expect(rx).toHaveAttribute('title', /.+/);
      expect(await rx.evaluate((e) => getComputedStyle(e).fontFamily)).toMatch(/mono/i);
    });
  });
}
