import { expect, Page, test } from '@playwright/test';
import { openView } from './app';
import { openAppWith } from './mock-url';

// A tab dragged out of the window: in Debrief the shell's ghost window follows the pointer outside it and the pop-out
// opens exactly where the ghost was (`drag_ghost.rs`). The browser build simulates that ghost with `?mockGhost=1`
// (`MockDragGhostBackend`, published as `window.__debriefMockGhost`); without it the in-window preview is used
// (dock-float.e2e).

interface GhostState {
  starts: number;
  ends: number;
  visible: boolean;
  rect: { x: number; y: number; width: number; height: number } | null;
  request: { title: string; width: number; height: number; grabX: number; grabY: number } | null;
}

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const ghost = (page: Page) =>
  page.evaluate(
    () =>
      JSON.parse(JSON.stringify((window as unknown as { __debriefMockGhost?: unknown }).__debriefMockGhost ?? { starts: 0, ends: 0, visible: false, rect: null, request: null })) as GhostState,
  );

test.describe('dragging a tab out of the window with the shell ghost', () => {
  test.beforeEach(async ({ page }) => {
    await openAppWith(page, '?mockGhost=1');
    await openView(page, 'metrics');
    // Records the features of the pop-out windows opened (their position and size).
    await page.evaluate(() => {
      const w = window as unknown as { __opens: string[] };
      w.__opens = [];
      const open = window.open.bind(window);
      window.open = (url?: string | URL, target?: string, features?: string) => {
        w.__opens.push(String(features));
        return open(url, target, features);
      };
    });
  });

  test('the ghost follows the pointer outside, hides back inside, and the pop-out opens exactly where it was', async ({ page, context }) => {
    await expect(tab(page, 'Metrics')).toBeVisible();
    const t = (await tab(page, 'Metrics').boundingBox())!;
    await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2);
    await page.mouse.down();
    await page.mouse.move(t.x + t.width / 2 - 40, t.y + 60, { steps: 5 });
    await page.mouse.move(300, -40, { steps: 5 });
    await expect.poll(async () => (await ghost(page)).visible).toBe(true);
    const first = await ghost(page);
    expect(first.request).toMatchObject({ title: 'Metrics', grabY: 12 });
    // The title bar is under the pointer: the ghost is the window that will open.
    expect(first.rect).toMatchObject({ width: first.request!.width, height: first.request!.height });
    // The ghost window shows the new window: the in-window preview is not drawn, its hint is still announced.
    await expect(page.locator('.debrief-float-cue')).toBeHidden();
    await expect(page.getByRole('status').filter({ hasText: 'Release to open in a new window' })).toHaveCount(1);
    await page.mouse.move(420, -60, { steps: 5 });
    await expect.poll(async () => (await ghost(page)).rect!.x).toBeGreaterThan(first.rect!.x + 100);
    // Back over the window, off the guides: the ghost hides and the cancel cue returns.
    const grid = (await page.locator('.dv-groupview:has(debrief-events-grid) .dv-content-container').boundingBox())!;
    await page.mouse.move(grid.x + grid.width * 0.3, grid.y + grid.height * 0.3, { steps: 5 });
    await expect.poll(async () => (await ghost(page)).visible).toBe(false);
    await expect(page.locator('.debrief-float-cue')).toHaveText('Release to cancel');
    // Out again and released there.
    await page.mouse.move(500, -40, { steps: 5 });
    await expect.poll(async () => (await ghost(page)).visible).toBe(true);
    const last = (await ghost(page)).rect!;
    const opened = context.waitForEvent('page');
    await page.mouse.up();
    const popup = await opened;
    await expect(popup.getByRole('heading', { level: 1, name: 'Metrics' })).toBeVisible();
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    const features = await page.evaluate(() => (window as unknown as { __opens: string[] }).__opens.at(-1));
    expect(features).toBe(`popup,left=${last.x},top=${last.y},width=${last.width},height=${last.height}`);
    expect(await ghost(page)).toMatchObject({ starts: 1, ends: 1, visible: false });
  });

  test('Escape outside the window hides the ghost and nothing pops out', async ({ page }) => {
    await expect(tab(page, 'Metrics')).toBeVisible();
    const t = (await tab(page, 'Metrics').boundingBox())!;
    await page.mouse.move(t.x + t.width / 2, t.y + t.height / 2);
    await page.mouse.down();
    await page.mouse.move(t.x + t.width / 2 - 40, t.y + 60, { steps: 5 });
    await page.mouse.move(300, -40, { steps: 5 });
    await expect.poll(async () => (await ghost(page)).visible).toBe(true);
    await page.keyboard.press('Escape');
    await expect.poll(async () => (await ghost(page)).ends).toBe(1);
    expect((await ghost(page)).visible).toBe(false);
    await page.mouse.up();
    await expect(tab(page, 'Metrics')).toBeVisible();
    expect(await page.evaluate(() => (window as unknown as { __opens: string[] }).__opens.length)).toBe(0);
  });
});
