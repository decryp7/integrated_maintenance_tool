import { expect, Page, test } from '@playwright/test';
import { ensureInspector, openApp, openView } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, menuItem, openMenu } from './menu-support';

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groups = (page: Page) => page.locator('.dv-groupview');
/** The group (tabs and its header actions) that holds the panel with this tab name. */
const groupOf = (page: Page, name: string) => groups(page).filter({ has: tab(page, name) });
/** The side-bar tab of the auto-hidden inspector (the default). */
const inspectorAhTab = (page: Page) => page.locator('.imt-ah-tab[data-ah-id="inspector"]');
const layout = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem('imt.layout') ?? 'null') as { version: number; layout: { panels: Record<string, unknown>; floatingGroups?: unknown[] }; extras?: unknown } | null);

async function box(page: Page, selector: string) {
  const b = await page.locator(selector).first().boundingBox();
  if (!b) throw new Error(`no box for ${selector}`);
  return b;
}

test.describe('dock layout', () => {
  // The docking mechanics are checked on the default arrangement (timeline and registry beside it, over the events).
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  test('default arrangement: timeline and registry over the events, the inspector auto-hidden on the right; it slides out on selection and pins', async ({ page }) => {
    const timeline = await box(page, 'imt-timeline-panel');
    const events = await box(page, 'imt-events-grid');
    expect(timeline.y + timeline.height).toBeLessThanOrEqual(events.y + 1);
    await expect(groups(page)).toHaveCount(3);
    for (const name of ['Timeline', 'Registry']) await expect(tab(page, name)).toBeVisible();
    // Metrics, the profile editor and the analysis and tool windows open on demand.
    for (const name of ['Metrics', 'Profile']) await expect(tab(page, name)).toHaveCount(0);
    await expect(tab(page, 'Timeline')).toHaveAttribute('aria-selected', 'true');
    await expect(inspectorAhTab(page)).toBeVisible();
    await expect(page.locator('imt-inspector')).toHaveCount(0);

    // Selecting an event slides the inspector out over the right edge.
    const rows = page.locator('imt-events-grid [role=row].row');
    await rows.first().click();
    const flyout = page.locator('#imt-ah-flyout');
    await expect(flyout.locator('imt-inspector')).toBeVisible();
    const slid = await box(page, '#imt-ah-flyout');
    expect(slid.width).toBeGreaterThan(300);
    expect(slid.x + slid.width).toBeGreaterThanOrEqual(events.x + events.width - 2);
    // Selecting another event keeps it open.
    await rows.nth(1).click();
    await expect(flyout.locator('imt-inspector')).toBeVisible();

    // Pinning docks it as a tool window on the right, and that choice is kept in the layout.
    await flyout.getByRole('button', { name: 'Pin Inspector' }).click();
    await expect(inspectorAhTab(page)).toHaveCount(0);
    await expect(groups(page)).toHaveCount(4);
    await expect
      .poll(async () => {
        const inspector = await box(page, 'imt-inspector');
        const grid = await box(page, 'imt-events-grid');
        return inspector.x - (grid.x + grid.width);
      })
      .toBeGreaterThanOrEqual(-2);
    await expect.poll(async () => Object.keys((await layout(page))?.layout.panels ?? {}).includes('inspector'), { timeout: 5000 }).toBe(true);
  });

  test('dragging a tab onto the right guide of another group splits the layout', async ({ page }) => {
    // Tabs are dragged with pointer events (dockview `dndStrategy: 'pointer'`), so the real drag runs in WebKit too.
    await openView(page, 'metrics');
    await expect(groups(page)).toHaveCount(3);
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
    await expect(groups(page)).toHaveCount(4);
    // Metrics now sits in its own group, to the right of the events grid.
    const metrics = await box(page, 'imt-metrics-view');
    const events = await box(page, 'imt-events-grid');
    expect(metrics.x).toBeGreaterThanOrEqual(events.x + events.width - 2);
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'true');
  });

  test('a group can be floated inside the window and closed', async ({ page }) => {
    await groupOf(page, 'Events').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Float', exact: true }).click();
    const floating = page.locator('.dv-resize-container');
    await expect(floating).toBeVisible();
    await expectNoSeriousViolations(page, test.info(), 'floating');
    // Dock it back.
    await floating.getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Dock', exact: true }).click();
    await expect(floating).toHaveCount(0);
    await expect(groups(page)).toHaveCount(3);
    // Float again and close its only tab with the keyboard.
    await ensureInspector(page);
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Float', exact: true }).click();
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
    // Auto-hidden counts as open: the item closes it.
    await expect(inspectorItem).toHaveAttribute('aria-checked', 'true');
    await inspectorItem.click();
    await expect(inspectorAhTab(page)).toHaveCount(0);
    await expect(groups(page)).toHaveCount(3);

    await expect(menuItem(await openMenu(page, 'View'), 'Inspector')).toHaveAttribute('aria-checked', 'false');
    await page.keyboard.press('Escape');
    await openView(page, 'metrics');
    await chooseMenu(page, 'Window', 'Perspectives', 'Reset perspective');
    await expect(inspectorAhTab(page)).toBeVisible();
    await expect(groups(page)).toHaveCount(3);
    await expect(tab(page, 'Metrics')).toHaveCount(0);
  });

  test('the layout persists across a reload, and an incompatible saved layout is ignored', async ({ page }) => {
    await ensureInspector(page);
    await openView(page, 'metrics');
    await tab(page, 'Metrics').focus();
    await page.keyboard.press('Delete');
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    const saved = async () => Object.keys((await layout(page))?.layout.panels ?? {});
    await expect.poll(async () => (await saved()).includes('inspector') && !(await saved()).includes('metrics'), { timeout: 5000 }).toBe(true);
    expect((await layout(page))?.version).toBe(2);

    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(tab(page, 'Registry')).toBeVisible();
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    // The pinned (docked) inspector is remembered.
    await expect(page.locator('imt-inspector')).toBeVisible();
    await expect(inspectorAhTab(page)).toHaveCount(0);

    // Seed before the app starts so a pending save of the running page cannot overwrite it.
    await page.addInitScript(() => localStorage.setItem('imt.layout', JSON.stringify({ version: 999, layout: {} })));
    await page.reload();
    await expect(inspectorAhTab(page)).toBeVisible();
    await expect(page.locator('imt-inspector')).toHaveCount(0);
  });

  test('tabs are keyboard accessible', async ({ page }) => {
    await openView(page, 'metrics');
    await tab(page, 'Registry').focus();
    await expect(tab(page, 'Registry')).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(tab(page, 'Metrics')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(tab(page, 'Metrics')).toHaveAttribute('aria-selected', 'true');
    await expectNoSeriousViolations(page, test.info(), 'docked-metrics');
  });
});
