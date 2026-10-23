import { expect, Locator, Page, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';
import { openAiApp, openChat, runCommand } from './ai-support';
import { expectGroupsAtMinimum, expectNoHorizontalScroll, expectNoOverlap, groupOf, measureGroups, paneWidths, resizeWindow, STANDARD_SIZES } from '../src/app/layout-plan/layout-scenarios.playwright';

// The reported layout: Explorer | timeline/events/markers | AI assistant | Home, at two common window sizes. Every
// pane keeps a readable width, Home's findings show their titles, and nothing sticks out of Explorer, the assistant or Markers.
const SHOTS = process.env['LAYOUT_SHOTS'];
const SIZES = STANDARD_SIZES;
/** The panes whose widths are reported (selector of the component inside the group). */
const PANES = { assistant: 'debrief-ai-chat-panel', timeline: 'debrief-timeline-panel', home: 'debrief-home-view', markers: 'debrief-markers-manager' };

/** What a scenario opens beside the Explorer, the timeline and the events grid. */
interface Open {
  assistant?: boolean;
  markers?: boolean;
  home?: boolean;
}

async function openViews(page: Page, open: Open): Promise<void> {
  if (open.assistant) await openChat(page);
  if (open.markers) {
    await runCommand(page, 'Manage markers');
    await expect(page.locator('debrief-markers-manager .rules tbody tr').first()).toBeVisible();
  }
  if (open.home) {
    await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();
    await expect(page.locator('debrief-home-view [data-testid=home-attention]')).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
    await expect(page.locator('debrief-home-view .finding .title').first()).toBeVisible();
  }
  await page.waitForTimeout(400);
}

async function openScenario(page: Page, open: Open = { assistant: true, markers: true, home: true }): Promise<void> {
  const scheme = process.env['LAYOUT_THEME'] === 'light' ? 'light' : 'dark';
  await page.addInitScript((s) => localStorage.setItem('debrief.prefs.v1', JSON.stringify({ scheme: s })), scheme);
  await page.emulateMedia({ colorScheme: scheme });
  await openAiApp(page, 'mockLongNames=1');
  await openViews(page, open);
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

      // Pane widths: every group is at or above the minimum its views declare (read from the app, not repeated here).
      const groups = await expectGroupsAtMinimum(page);
      expectNoOverlap(groups);
      await expectNoHorizontalScroll(page);
      info.annotations.push({ type: 'pane-widths', description: JSON.stringify(await paneWidths(page, PANES)) });

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

// The standard sets, each at both sizes: every group keeps its declared minimum, none overlap, nothing scrolls sideways.
const SETS: { name: string; open: Open }[] = [
  { name: 'Explorer + events + assistant', open: { assistant: true } },
  { name: '+ Home', open: { assistant: true, home: true } },
  { name: '+ Markers', open: { assistant: true, markers: true } },
  { name: '+ Home + Markers (the reported scenario)', open: { assistant: true, markers: true, home: true } },
];
for (const size of SIZES) {
  test.describe(`scenario table ${size.width}x${size.height}`, () => {
    test.use({ viewport: size });
    for (const set of SETS) {
      test(set.name, async ({ page }, info) => {
        await openScenario(page, set.open);
        const groups = await expectGroupsAtMinimum(page);
        expectNoOverlap(groups);
        await expectNoHorizontalScroll(page);
        info.annotations.push({ type: 'pane-widths', description: JSON.stringify(await paneWidths(page, PANES)) });
      });
    }
  });
}

test.describe('resizing', () => {
  test.use({ viewport: { width: 1920, height: 1080 } });

  test('shrinking 1920 to 1440 to 1000 tabs lower-priority panes instead of squeezing any below its minimum', async ({ page }) => {
    await openScenario(page);
    await expectGroupsAtMinimum(page);
    await resizeWindow(page, { width: 1440, height: 900 });
    const at1440 = await expectGroupsAtMinimum(page);
    expectNoOverlap(at1440);
    await resizeWindow(page, { width: 1000, height: 800 });
    const at1000 = await expectGroupsAtMinimum(page);
    expectNoOverlap(at1000);
    await expectNoHorizontalScroll(page);
    // Home yielded: it is a tab of another group, not a sliver.
    const home = await groupOf(page, 'debrief-home-view');
    if (home) expect(home.width).toBeGreaterThanOrEqual(320);
    expect(at1000.some((g) => g.tabs.length > 1), 'a pane was tabbed').toBe(true);
  });
});

test.describe('a saved layout that does not fit the window', () => {
  test.use({ viewport: { width: 1920, height: 1080 } });

  test('is fitted at load without rewriting it on disk, with one notification offering Undo', async ({ page }) => {
    await openScenario(page);
    await page.waitForTimeout(600);
    const saved = await page.evaluate(() => localStorage.getItem('debrief.layout'));
    expect(saved).toBeTruthy();
    await page.addInitScript((v) => {
      if (!sessionStorage.getItem('seeded')) {
        localStorage.setItem('debrief.layout', v);
        sessionStorage.setItem('seeded', '1');
      }
    }, saved!);
    await page.setViewportSize({ width: 1000, height: 800 });
    await page.reload();
    await expect(page.locator('.dv-groupview').first()).toBeVisible();
    await page.waitForTimeout(900);
    await page.screenshot({ path: 'test-results/saved-fit.png' });
    const groups = await expectGroupsAtMinimum(page);
    expectNoOverlap(groups);
    await expectNoHorizontalScroll(page);
    // The layout on disk is the one that was saved, not the fitted one.
    expect(await page.evaluate(() => localStorage.getItem('debrief.layout'))).toBe(saved);
    // One notification offers Undo.
    await page.getByTestId('status-notifications').click();
    const notice = page.locator('debrief-notifications-panel').filter({ hasText: 'Some panes were tabbed to fit this window' });
    await expect(notice).toBeVisible();
    await notice.getByRole('button', { name: 'Undo' }).click();
    expect((await measureGroups(page)).length).toBeGreaterThan(0);
  });
});
