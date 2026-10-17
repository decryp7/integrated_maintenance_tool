import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, menuItem, openMenu } from './menu-support';

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groups = (page: Page) => page.locator('.dv-groupview');
/** The group (tabs and its header actions) that holds the panel with this tab name. */
const groupOf = (page: Page, name: string) => groups(page).filter({ has: tab(page, name) });
const layout = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem('imt.layout') ?? 'null') as { version: number; layout: { panels: Record<string, unknown>; floatingGroups?: unknown[] }; extras?: unknown } | null);

async function box(page: Page, selector: string) {
  const b = await page.locator(selector).first().boundingBox();
  if (!b) throw new Error(`no box for ${selector}`);
  return b;
}

test.describe('dock layout', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('default layout: timeline over the query bar over events, inspector on the right, other views stacked with the timeline', async ({ page }) => {
    const timeline = await box(page, 'imt-timeline-panel');
    const query = await box(page, 'imt-query-view');
    const events = await box(page, 'imt-events-grid');
    const inspector = await box(page, 'imt-inspector');
    expect(timeline.y + timeline.height).toBeLessThanOrEqual(query.y + 1);
    expect(query.y + query.height).toBeLessThanOrEqual(events.y + 1);
    expect(inspector.x).toBeGreaterThanOrEqual(Math.max(timeline.x + timeline.width, events.x + events.width) - 2);
    expect(inspector.width).toBeGreaterThan(300);
    await expect(groups(page)).toHaveCount(4);
    for (const name of ['Timeline', 'Metrics', 'Registry']) await expect(tab(page, name)).toBeVisible();
    // The profile editor and the analysis and tool windows open on demand.
    await expect(tab(page, 'Profile')).toHaveCount(0);
    await expect(tab(page, 'Timeline')).toHaveAttribute('aria-selected', 'true');
  });

  test('dragging a tab onto the right guide of another group splits the layout', async ({ page }) => {
    // Tabs are dragged with pointer events (dockview `dndStrategy: 'pointer'`), so the real drag runs in WebKit too.
    await expect(groups(page)).toHaveCount(4);
    const source = tab(page, 'Metrics');
    // The right cell of the VS-style compass in the middle of the events group (see dock-geometry `compassCells`).
    const c = (await page.locator('.dv-groupview:has(imt-events-grid) .dv-content-container').boundingBox())!;
    const right = { x: c.x + c.width / 2 + 36, y: c.y + c.height / 2 };
    const s = (await source.boundingBox())!;
    await page.mouse.move(s.x + s.width / 2, s.y + s.height / 2);
    await page.mouse.down();
    await page.mouse.move(s.x + s.width / 2 + 20, s.y + s.height / 2 + 20, { steps: 5 });
    await page.mouse.move(right.x - 6, right.y, { steps: 15 });
    await page.mouse.move(right.x, right.y, { steps: 3 });
    await page.mouse.up();
    await expect(groups(page)).toHaveCount(5);
    // Metrics now sits in its own group, to the right of the events grid.
    const metrics = await box(page, 'imt-metrics-view');
    const events = await box(page, 'imt-events-grid');
    expect(metrics.x).toBeGreaterThanOrEqual(events.x + events.width - 2);
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'true');
  });

  test('a group can be floated inside the window and closed', async ({ page }) => {
    await groupOf(page, 'Events').getByRole('button', { name: 'Float panel' }).click();
    const floating = page.locator('.dv-resize-container');
    await expect(floating).toBeVisible();
    await expectNoSeriousViolations(page, test.info(), 'floating');
    // Dock it back.
    await page.getByRole('button', { name: 'Dock panel back into the window' }).click();
    await expect(floating).toHaveCount(0);
    await expect(groups(page)).toHaveCount(4);
    // Float again and close its only tab with the keyboard.
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Float panel' }).click();
    await expect(floating).toBeVisible();
    await floating.getByRole('tab').first().focus();
    await page.keyboard.press('Delete');
    await expect(floating).toHaveCount(0);
  });

  test('a group can be maximized and restored', async ({ page }) => {
    await groupOf(page, 'Events').getByRole('button', { name: 'Maximize panel' }).click();
    await expect(page.getByRole('button', { name: 'Restore panel size' })).toBeVisible();
    const grid = await box(page, 'imt-events-grid');
    expect(grid.width).toBeGreaterThan(1000);
    await page.getByRole('button', { name: 'Restore panel size' }).click();
    await expect(page.getByRole('button', { name: 'Restore panel size' })).toHaveCount(0);
  });

  test('panels can be toggled from the View menu and the layout reset', async ({ page }) => {
    const inspectorItem = menuItem(await openMenu(page, 'View'), 'Inspector');
    await expect(inspectorItem).toHaveAttribute('aria-checked', 'true');
    await inspectorItem.click();
    await expect(page.locator('imt-inspector')).toHaveCount(0);
    await expect(groups(page)).toHaveCount(3);

    await expect(menuItem(await openMenu(page, 'View'), 'Inspector')).toHaveAttribute('aria-checked', 'false');
    await page.keyboard.press('Escape');
    await chooseMenu(page, 'Window', 'Layouts', 'Reset window layout');
    await expect(page.locator('imt-inspector')).toBeVisible();
    await expect(groups(page)).toHaveCount(4);
    await expect(tab(page, 'Metrics')).toBeVisible();
  });

  test('the layout persists across a reload, and an incompatible saved layout is ignored', async ({ page }) => {
    await tab(page, 'Metrics').focus();
    await page.keyboard.press('Delete');
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    await expect.poll(async () => (await layout(page)) !== null && !Object.keys((await layout(page))!.layout.panels).includes('metrics'), { timeout: 5000 }).toBe(true);
    expect((await layout(page))?.version).toBe(2);

    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(tab(page, 'Registry')).toBeVisible();
    await expect(tab(page, 'Metrics')).toHaveCount(0);

    // Seed before the app starts so a pending save of the running page cannot overwrite it.
    await page.addInitScript(() => localStorage.setItem('imt.layout', JSON.stringify({ version: 999, layout: {} })));
    await page.reload();
    await expect(tab(page, 'Metrics')).toBeVisible();
  });

  test('tabs are keyboard accessible', async ({ page }) => {
    await tab(page, 'Timeline').focus();
    await expect(tab(page, 'Timeline')).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(tab(page, 'Metrics')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'true');
    await expectNoSeriousViolations(page, test.info(), 'docked-metrics');
  });
});
