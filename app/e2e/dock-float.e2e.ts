import { expect, Page, test } from '@playwright/test';
import { openApp, openView } from './app';
import { chooseMenu } from './menu-support';

// Dragging a tab: a release on a drop guide docks it; a release inside the window but off every guide cancels the
// drag ("Release to cancel" by the pointer; the tab stays, nothing floats); outside the window a translucent ghost labelled "Release to open in a
// new window" follows the pointer (popout.e2e covers the release there). Floating inside the window is deliberate:
// the tab menu's "Float", the group's ⋯ panel menu or Window ▸ Float panel. Tabs are dragged
// with pointer events.

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const cue = (page: Page) => page.locator('.debrief-float-cue');
const floating = (page: Page) => page.locator('.dv-resize-container');
const groupOf = (page: Page, name: string) => page.locator('.dv-groupview').filter({ has: tab(page, name) });

/** The content box of the events grid's group. */
async function eventsBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  return (await page.locator('.dv-groupview:has(debrief-events-grid) .dv-content-container').boundingBox())!;
}

/** A point over the events grid well away from its compass (the middle) and the dock's edge guides. */
async function freeSpot(page: Page): Promise<{ x: number; y: number }> {
  const c = await eventsBox(page);
  return { x: c.x + c.width * 0.3, y: c.y + c.height * 0.3 };
}

async function pressAndDrag(page: Page, name: string): Promise<void> {
  const s = (await tab(page, name).boundingBox())!;
  await page.mouse.move(s.x + s.width / 2, s.y + s.height / 2);
  await page.mouse.down();
  await page.mouse.move(s.x + s.width / 2 + 20, s.y + s.height / 2 + 20, { steps: 5 });
}

/** Drags the floating window by the empty part of its tab strip (as one moves a window by its title bar). */
async function moveFloatingByHeader(page: Page): Promise<void> {
  const before = (await floating(page).boundingBox())!;
  const header = (await floating(page).locator('.dv-void-container').boundingBox())!;
  const start = { x: header.x + header.width / 2, y: header.y + header.height / 2 };
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(start.x - 60, start.y - 50, { steps: 10 });
  // Moving a window docks nothing: no dock guides.
  await expect(page.locator('.debrief-drop-guides.on')).toHaveCount(0);
  await page.mouse.up();
  await expect.poll(async () => (await floating(page).boundingBox())!.x).toBeLessThan(before.x - 40);
  await expect.poll(async () => (await floating(page).boundingBox())!.y).toBeLessThan(before.y - 30);
}

test.describe('dragging a panel tab', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
    await openView(page, 'metrics');
  });

  test('released on nothing inside the window the drag is cancelled: "Release to cancel", nothing floats, the tab stays', async ({ page }) => {
    const before = await groupOf(page, 'Metrics').locator('[role="tab"]').allInnerTexts();
    await pressAndDrag(page, 'Metrics');
    const free = await freeSpot(page);
    await page.mouse.move(free.x, free.y, { steps: 10 });
    await expect(cue(page)).toHaveText('Release to cancel');
    await expect(page.getByRole('status').filter({ hasText: 'Release to cancel' })).toHaveCount(1);
    // The hint sits by the pointer, and the dragged tab chip dims.
    const hint = (await cue(page).boundingBox())!;
    expect(Math.abs(hint.x - free.x)).toBeLessThan(40);
    expect(Math.abs(hint.y - free.y)).toBeLessThan(80);
    await expect(page.locator('body.debrief-drag-cancel')).toHaveCount(1);
    await page.mouse.up();
    await expect(cue(page)).toBeHidden();
    await expect(page.locator('body.debrief-drag-cancel')).toHaveCount(0);
    await expect(floating(page)).toHaveCount(0);
    expect(await groupOf(page, 'Metrics').locator('[role="tab"]').allInnerTexts()).toEqual(before);
    await expect(groupOf(page, 'Events')).not.toContainText('Metrics');
  });

  test('released on a drop guide it docks there', async ({ page }) => {
    await pressAndDrag(page, 'Metrics');
    const c = await eventsBox(page);
    await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2, { steps: 15 });
    await page.mouse.move(c.x + c.width / 2, c.y + c.height / 2 + 1, { steps: 2 });
    await expect(page.locator('.debrief-guide-center.hot')).toHaveCount(1);
    await expect(cue(page)).toBeHidden();
    await page.mouse.up();
    await expect(groupOf(page, 'Events')).toContainText('Metrics');
    await expect(floating(page)).toHaveCount(0);
  });

  test('outside the window a translucent ghost follows the pointer with "Release to open in a new window"', async ({ page }) => {
    await pressAndDrag(page, 'Metrics');
    const free = await freeSpot(page);
    await page.mouse.move(free.x, -40, { steps: 10 });
    await expect(cue(page)).toBeVisible();
    await expect(cue(page)).toHaveText('Release to open in a new window');
    await expect(page.getByRole('status').filter({ hasText: 'Release to open in a new window' })).toHaveCount(1);
    expect(Number(await cue(page).evaluate((el) => getComputedStyle(el).opacity))).toBeLessThan(1);
    // Held at the window's edge, under the pointer's x.
    const first = (await cue(page).boundingBox())!;
    expect(first.y).toBe(0);
    expect(free.x).toBeGreaterThanOrEqual(first.x);
    expect(free.x).toBeLessThanOrEqual(first.x + first.width);
    await page.mouse.move(free.x + 150, -40, { steps: 10 });
    await expect.poll(async () => (await cue(page).boundingBox())!.x).toBeGreaterThan(first.x + 100);
    // Back inside, off the guides: releasing there cancels.
    await page.mouse.move(free.x, free.y, { steps: 10 });
    await expect(cue(page)).toHaveText('Release to cancel');
    await page.mouse.up();
    await expect(floating(page)).toHaveCount(0);
    await expect(tab(page, 'Metrics')).toBeVisible();
  });

  test('Escape cancels a drag outside the window: the cue clears and nothing pops out', async ({ page }) => {
    await pressAndDrag(page, 'Metrics');
    const free = await freeSpot(page);
    await page.mouse.move(free.x, -40, { steps: 10 });
    await expect(cue(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(cue(page)).toBeHidden();
    await page.mouse.up();
    await expect(floating(page)).toHaveCount(0);
    await expect(tab(page, 'Metrics')).toBeVisible();
  });
});

test.describe('floating a panel deliberately', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
    await openView(page, 'metrics');
  });

  test('from the tab menu ("Float"), then moved by its header', async ({ page }) => {
    await tab(page, 'Metrics').click({ button: 'right' });
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Float', exact: true }).click();
    await expect(floating(page).getByRole('tab', { name: 'Metrics', exact: true })).toBeVisible();
    await moveFloatingByHeader(page);
  });

  test('from the group panel menu (⋯ ▸ Float), then moved by its header', async ({ page }) => {
    await page.locator('.dv-groupview', { has: tab(page, 'Metrics') }).getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Float', exact: true }).click();
    await expect(floating(page)).toBeVisible();
    await moveFloatingByHeader(page);
  });

  test('from Window ▸ Float panel (the active panel)', async ({ page }) => {
    await tab(page, 'Metrics').click();
    await chooseMenu(page, 'Window', 'Float panel');
    await expect(floating(page).getByRole('tab', { name: 'Metrics', exact: true })).toBeVisible();
  });
});
