import { expect, Locator, Page, test } from '@playwright/test';
import { openApp } from './app';

/** The dock group (`.dv-groupview`) holding the panel inside `inner`. */
function groupOf(inner: Locator): Locator {
  return inner.locator('xpath=ancestor::*[contains(@class,"dv-groupview")][1]');
}

async function box(l: Locator) {
  const b = await l.boundingBox();
  if (!b) throw new Error('not laid out');
  return b;
}

/** Home and Registry are documents that need height: their group spans the dock, not the 300 px timeline row. */
async function expectFullHeight(page: Page, panel: Locator): Promise<void> {
  const dock = await box(page.locator('debrief-dock-host'));
  const group = await box(groupOf(panel));
  expect(group.height).toBeGreaterThan(dock.height * 0.95);
  // The grid-space fix stays: the events grid and the timeline keep their rows beside the column.
  const events = await box(groupOf(page.locator('debrief-events-grid')));
  const timeline = await box(groupOf(page.locator('debrief-timeline-panel')));
  expect(events.height).toBeGreaterThan(200);
  expect(timeline.height).toBeGreaterThan(150);
  expect(events.height + timeline.height).toBeGreaterThan(dock.height * 0.9);
  expect(group.width).toBeGreaterThan(400);
}

test.describe('full-height documents (Home, Registry)', () => {
  test('Registry opens as a column as tall as the dock, beside the timeline and the grid', async ({ page }) => {
    await openApp(page);
    await page.getByRole('tab', { name: 'Registry', exact: true }).click();
    const tree = page.getByRole('tree', { name: 'Registry' });
    await expect(tree).toBeVisible();
    await expectFullHeight(page, tree);
  });

  test('Home opens as a column as tall as the dock, and shares it with Registry as a tab', async ({ page }) => {
    await openApp(page);
    await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();
    const home = page.locator('debrief-home-view');
    await expect(home).toBeVisible({ timeout: 10_000 });
    await expectFullHeight(page, home);
    const registryTab = page.getByRole('tab', { name: 'Registry', exact: true });
    await registryTab.click();
    await expect(page.getByRole('tree', { name: 'Registry' })).toBeVisible();
    // One group holds both full-height views: Home's tab is in the same group box.
    const homeGroup = await box(groupOf(page.getByRole('tab', { name: 'Home', exact: true })));
    expect(await box(groupOf(registryTab))).toEqual(homeGroup);
  });
});

test.describe('2560 px window (U15) and the explorer (U14)', () => {
  test.use({ viewport: { width: 2560, height: 1440 } });

  test('the explorer names the shared folder once, nothing overflows, and full-height documents stay narrow', async ({ page }) => {
    await openApp(page);
    const sidebar = page.locator('debrief-sidebar');
    const root = sidebar.locator('[data-testid=folder-root]');
    await expect(root).toBeVisible();
    const rootPath = (await root.getAttribute('title')) ?? '';
    expect(rootPath.length).toBeGreaterThan(0);
    const headings = await sidebar.locator('[data-testid=folder-heading]').allTextContents();
    expect(headings.length).toBeGreaterThan(1);
    // Hoisted: no heading repeats the shared folder's last segment.
    const rootName = rootPath.split('/').filter(Boolean).pop() ?? '';
    for (const h of headings) expect(h.trim()).not.toContain(rootName);

    // The side bar keeps its width on a wide window and no row is wider than it.
    const side = await box(sidebar);
    expect(side.width).toBeLessThanOrEqual(400);
    const overflowing = await sidebar.locator('.src').evaluateAll((els) => els.filter((e) => e.scrollWidth > e.clientWidth + 1).length);
    expect(overflowing).toBe(0);
    const grid = await box(page.locator('debrief-events-grid'));
    expect(grid.x + grid.width).toBeLessThanOrEqual(2560);

    await page.getByRole('tab', { name: 'Registry', exact: true }).click();
    await expect(page.getByRole('tree', { name: 'Registry' })).toBeVisible();
    const column = await box(groupOf(page.getByRole('tree', { name: 'Registry' })));
    expect(column.width).toBeLessThanOrEqual(880 + 4);
    expect(column.height).toBeGreaterThan(1000);
  });
});
