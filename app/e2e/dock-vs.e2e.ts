import { expect, Locator, Page, test } from '@playwright/test';
import { ensureInspector, openApp, openView } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';
import { showProfileEditor } from './menu-support';

// Visual Studio-style docking: drop guides, auto hide, the panel menu, pinned tabs, the Ctrl+Tab navigator and
// named perspectives.

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groups = (page: Page) => page.locator('.dv-groupview');
const groupOf = (page: Page, name: string) => groups(page).filter({ has: tab(page, name) });

async function box(page: Page, selector: string) {
  // A panel moving between the dock and the auto-hide fly-out is re-mounted: wait until it has a box again.
  let b: Awaited<ReturnType<Locator['boundingBox']>> = null;
  await expect.poll(async () => (b = await page.locator(selector).filter({ visible: true }).first().boundingBox())).not.toBeNull();
  if (!b) throw new Error(`no box for ${selector}`);
  return b;
}

/** Starts a native drag of a tab and moves it to (x, y) without releasing. */
async function dragTabTo(page: Page, name: string, x: number, y: number): Promise<void> {
  const s = (await tab(page, name).boundingBox())!;
  await page.mouse.move(s.x + s.width / 2, s.y + s.height / 2);
  await page.mouse.down();
  await page.mouse.move(s.x + s.width / 2 + 20, s.y + s.height / 2 + 20, { steps: 5 });
  await page.mouse.move(x, y, { steps: 15 });
  await page.mouse.move(x, y + 1, { steps: 2 });
}

/** Opens the panel menu of a tab (right click). */
async function panelMenu(page: Page, name: string) {
  await tab(page, name).click({ button: 'right' });
  const menu = page.getByRole('menu', { name: 'Actions' });
  await expect(menu).toBeVisible();
  return menu;
}

test.describe('VS-style docking', () => {
  // Wide enough for a document column, the timeline column and a tool window side by side (at 1440 px the document is tabbed into the events group instead).
  test.use({ viewport: { width: 1920, height: 1080 } });
  // These start from the fuller arrangement (the inspector docked, Metrics tabbed with the timeline) that the
  // minimal default no longer opens.
  // The docking mechanics are checked on the default arrangement (timeline and the registry beside it over the events).
  test.beforeEach(async ({ page }) => {
    await openApp(page);
    await ensureInspector(page);
    await openView(page, 'metrics');
  });

  test('dragging a tab onto the left edge guide docks it along the whole left edge', async ({ page }) => {
    const dock = await box(page, '.debrief-dock-mount');
    // A point of the left guide (32px, centred) that is not on a group's tab strip: a release over a tab strip tabs the
    // panel into that group even while the edge guide is lit (see the R5 report).
    const x = dock.x + 10 + 16;
    const y = await page.evaluate(
      ({ x, mid }) => {
        for (let d = 0; d <= 14; d += 2) for (const y of [mid - d, mid + d]) if (!document.elementFromPoint(x, y)?.closest('.dv-tabs-and-actions-container')) return y;
        return mid;
      },
      { x, mid: dock.y + dock.height / 2 },
    );
    await dragTabTo(page, 'Metrics', x, y);
    await expect(page.locator('.debrief-guide-left.hot')).toHaveCount(1);
    await expect(page.locator('.debrief-guide-edges .debrief-guide')).toHaveCount(4);
    await page.mouse.up();
    const metrics = await box(page, 'debrief-metrics-view');
    const events = await box(page, 'debrief-events-grid');
    expect(metrics.x).toBeLessThan(events.x);
    // Full height: from above the timeline to below the events.
    expect(metrics.height).toBeGreaterThan(events.height + 200);
  });

  test('dragging a tab onto the centre of the compass tabs it into that group', async ({ page }) => {
    const content = await box(page, '.dv-groupview:has(debrief-events-grid) .dv-content-container');
    await dragTabTo(page, 'Metrics', content.x + content.width / 2, content.y + content.height / 2);
    await expect(page.locator('.debrief-guide-center.hot')).toHaveCount(1);
    await page.mouse.up();
    await expect(groupOf(page, 'Events')).toContainText('Metrics');
    await expect(groups(page)).toHaveCount(4);
  });

  test('a released drag away from any guide is cancelled: the panel stays where it was and nothing floats', async ({ page }) => {
    const before = await groupOf(page, 'Metrics').locator('[role="tab"]').allInnerTexts();
    const content = await box(page, '.dv-groupview:has(debrief-events-grid) .dv-content-container');
    await dragTabTo(page, 'Metrics', content.x + 40, content.y + content.height - 20);
    await expect(page.locator('.debrief-float-cue')).toHaveText('Release to cancel');
    await page.mouse.up();
    await expect(page.locator('.dv-resize-container')).toHaveCount(0);
    await expect(groupOf(page, 'Events')).not.toContainText('Metrics');
    expect(await groupOf(page, 'Metrics').locator('[role="tab"]').allInnerTexts()).toEqual(before);
  });

  test('auto hide: the inspector collapses to a side bar, slides out on hover, hides on Escape, pins back', async ({ page }) => {
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto hide', exact: true }).click();
    await expect(tab(page, 'Inspector')).toHaveCount(0);
    const sideTab = page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' });
    await expect(sideTab).toBeVisible();
    await expect(groups(page)).toHaveCount(4);

    await sideTab.hover();
    const flyout = page.getByRole('region', { name: 'Inspector' });
    await expect(flyout).toBeVisible();
    await expect(flyout.locator('debrief-inspector')).toBeVisible();
    await expect(sideTab).toHaveAttribute('aria-expanded', 'true');
    await flyout.evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));
    await expectNoSeriousViolations(page, test.info(), 'auto-hide');
    await flyout.locator('debrief-inspector').click();
    await page.keyboard.press('Escape');
    await expect(flyout).toHaveCount(0);

    // Keyboard: focus the side tab and press Enter.
    await sideTab.focus();
    await page.keyboard.press('Enter');
    await expect(flyout).toBeVisible();
    await expect(flyout).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(flyout).toHaveCount(0);
    await expect(sideTab).toBeFocused();

    // Pin: back where it was.
    await sideTab.click();
    await flyout.getByRole('button', { name: 'Pin Inspector' }).click();
    await expect(tab(page, 'Inspector')).toBeVisible();
    // Pinning re-plans the dock: at 1920 px the Explorer, the timeline beside Metrics, the Registry and the Inspector do not
    // fit side by side, so Metrics is tabbed into the timeline's group again, as at the start (four groups). The five-group
    // state in between is transient (macOS saw it by luck; a slower machine does not).
    await expect(groups(page)).toHaveCount(4);
    await expect(groupOf(page, 'Timeline').getByRole('tab', { name: 'Metrics', exact: true })).toBeVisible();
    await expect(page.locator('debrief-inspector')).toBeVisible();
    // Back at the right edge of the window, beside the groups it was beside, once the dock layout has settled.
    await expect
      .poll(async () => {
        const inspector = await box(page, 'debrief-inspector');
        const timeline = await box(page, 'debrief-timeline-panel');
        return inspector.x - (timeline.x + timeline.width);
      })
      .toBeGreaterThanOrEqual(-2);
    // A full-height column again (as tall as the timeline over the events), not a split beside the top row only.
    const column = await box(page, 'debrief-inspector');
    const top = await box(page, 'debrief-timeline-panel');
    const grid = await box(page, 'debrief-events-grid');
    expect(column.y).toBeLessThanOrEqual(top.y + 2);
    expect(column.y + column.height).toBeGreaterThanOrEqual(grid.y + grid.height - 2);
  });

  test('auto hide button: collapses a tool pane to its edge, chooses left, right or bottom, slides out and pins back', async ({ page }) => {
    // The documents (Events) offer no auto-hide button at all; the inspector's pane does, with its shortcut in the tooltip.
    await expect(groupOf(page, 'Events').getByRole('button', { name: /^Auto-hide/ })).toHaveCount(0);
    const button = groupOf(page, 'Inspector').getByRole('button', { name: 'Auto-hide Inspector' });
    await expect(button).toHaveAttribute('aria-pressed', 'false');
    await expect(button).toHaveAttribute('title', /Ctrl\+Alt\+H|⌃⌥H/);

    // Default: the edge the pane is nearest to (right).
    await button.click();
    await expect(tab(page, 'Inspector')).toHaveCount(0);
    const right = page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' });
    await expect(right).toBeVisible();
    await right.click();
    const flyout = page.getByRole('region', { name: 'Inspector' });
    await expect(flyout).toBeVisible();
    const pin = flyout.getByRole('button', { name: 'Pin Inspector' });
    await expect(pin).toHaveAttribute('aria-pressed', 'true');
    await pin.click();
    await expect(tab(page, 'Inspector')).toBeVisible();

    // The choice (right-click or Shift+F10 on the button): left, right or bottom, never the top.
    await button.click({ button: 'right' });
    const menu = page.getByRole('menu', { name: 'Actions' });
    await expect(menu.getByRole('menuitem')).toHaveText(['Auto-hide to the left', 'Auto-hide to the right', 'Auto-hide to the bottom']);
    await menu.getByRole('menuitem', { name: 'Auto-hide to the bottom' }).click();
    const bottom = page.getByRole('navigation', { name: 'Auto-hidden tool windows (bottom)' }).getByRole('button', { name: 'Inspector' });
    await expect(bottom).toBeVisible();
    await bottom.hover();
    await expect(flyout).toBeVisible();
    await flyout.getByRole('button', { name: 'Pin Inspector' }).click();
    await expect(tab(page, 'Inspector')).toBeVisible();

    // It remembers: the plain button now goes to the bottom again; the keyboard reaches the choice too.
    await button.click();
    await expect(bottom).toBeVisible();
    await bottom.click();
    await flyout.getByRole('button', { name: 'Pin Inspector' }).click();
    await button.focus();
    await page.keyboard.press('Shift+F10');
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto-hide to the left' }).click();
    await expect(page.getByRole('navigation', { name: 'Auto-hidden tool windows (left)' }).getByRole('button', { name: 'Inspector' })).toBeVisible();
  });

  test('the auto-hide command toggles the active tool window', async ({ page }) => {
    await tab(page, 'Inspector').click();
    await runCommand(page, 'Auto hide / pin tool window');
    await expect(page.getByRole('navigation', { name: /Auto-hidden tool windows/ })).toBeVisible();
    await runCommand(page, 'Pin all tool windows');
    await expect(tab(page, 'Inspector')).toBeVisible();
  });

  test('the panel menu floats, docks and closes; it is accessible', async ({ page }) => {
    const menu = await panelMenu(page, 'Events');
    await expect(menu.getByRole('menuitem', { name: 'Pop out to window' })).toBeEnabled();
    await expect(menu.getByRole('menuitem', { name: 'Close all but this' })).toBeVisible();
    await expectNoSeriousViolations(page, test.info(), 'panel-menu');
    await menu.getByRole('menuitem', { name: 'Float', exact: true }).click();
    await expect(page.locator('.dv-resize-container')).toBeVisible();
    await (await panelMenu(page, 'Events')).getByRole('menuitem', { name: 'Dock', exact: true }).click();
    await expect(page.locator('.dv-resize-container')).toHaveCount(0);
    // Keyboard: Shift+F10 on a focused tab.
    await tab(page, 'Inspector').focus();
    await page.keyboard.press('Shift+F10');
    const toolMenu = page.getByRole('menu', { name: 'Actions' });
    await expect(toolMenu.getByRole('menuitem', { name: 'Auto hide' })).toBeVisible();
    await expect(toolMenu.getByRole('menuitem', { name: 'Dock as tabbed document' })).toBeVisible();
    await page.keyboard.press('Escape');
  });

  test('a pinned tab stays left and survives "Close all documents"', async ({ page }) => {
    await showProfileEditor(page);
    await (await panelMenu(page, 'Profile')).getByRole('menuitem', { name: 'Pin tab' }).click();
    const tabs = groupOf(page, 'Profile').getByRole('tab');
    await expect(tabs.first()).toHaveAccessibleName('Profile');
    await expect(groupOf(page, 'Profile').locator('.debrief-tab.pinned')).toHaveCount(1);
    await (await panelMenu(page, 'Timeline')).getByRole('menuitem', { name: 'Close all documents' }).click();
    await expect(tab(page, 'Profile')).toBeVisible();
    for (const name of ['Timeline', 'Metrics', 'Events']) await expect(tab(page, name)).toHaveCount(0);
    await expect(tab(page, 'Inspector')).toBeVisible();
  });

  test('Ctrl+Tab opens the navigator; Tab moves while Ctrl is held; releasing Ctrl switches', async ({ page }) => {
    await tab(page, 'Events').click();
    await tab(page, 'Inspector').click();
    await page.keyboard.down('Control');
    await page.keyboard.press('Tab');
    const nav = page.getByRole('dialog', { name: 'Switch to panel' });
    await expect(nav).toBeVisible();
    await expect(nav.getByRole('option', { selected: true })).toHaveText('Events');
    await expectNoSeriousViolations(page, test.info(), 'switcher');
    await page.keyboard.press('Tab');
    const second = (await nav.getByRole('option', { selected: true }).innerText()).trim();
    await page.keyboard.up('Control');
    await expect(nav).toHaveCount(0);
    await expect(tab(page, second)).toHaveAttribute('aria-selected', 'true');
  });

  test('a saved perspective is rebuilt by Reset perspective', async ({ page }) => {
    await runCommand(page, 'Save current as perspective…');
    const name = page.getByRole('textbox', { name: 'Perspective name' });
    await name.fill('Triage');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(name).toHaveCount(0);
    await tab(page, 'Metrics').focus();
    await page.keyboard.press('Delete');
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto hide', exact: true }).click();
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    // Triage is the active perspective; resetting it rebuilds the arrangement that was saved.
    await runCommand(page, 'Reset perspective');
    await expect(tab(page, 'Metrics')).toBeVisible();
    await expect(tab(page, 'Inspector')).toBeVisible();
    await expect(page.getByRole('navigation', { name: /Auto-hidden tool windows/ })).toHaveCount(0);
    // Saved app-wide, separately from the project.
    expect(await page.evaluate(() => JSON.parse(localStorage.getItem('debrief.windowLayouts') ?? 'null')?.layouts?.[0]?.name)).toBe('Triage');
    await runCommand(page, 'Manage perspectives…');
    await expect(page.getByRole('list', { name: 'Perspectives' })).toContainText('Triage');
    await expectNoSeriousViolations(page, test.info(), 'manage-layouts');
  });

  test('Reset perspective restores the minimal default arrangement', async ({ page }) => {
    await (await panelMenu(page, 'Metrics')).getByRole('menuitem', { name: 'Pin tab' }).click();
    await runCommand(page, 'Reset perspective');
    // Timeline, registry and events; the inspector auto-hidden on the right; Metrics closed.
    await expect(tab(page, 'Inspector')).toHaveCount(0);
    await expect(page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' })).toBeVisible();
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    await expect(page.locator('.debrief-tab.pinned')).toHaveCount(0);
    await expect(groups(page)).toHaveCount(3);
  });

  test('full screen hides the chrome and the tool windows; again restores them', async ({ page }) => {
    await page.keyboard.press('Shift+Alt+Enter');
    await expect(page.locator('debrief-title-bar')).toBeHidden();
    await expect(page.locator('debrief-inspector')).toBeHidden();
    await page.keyboard.press('Shift+Alt+Enter');
    await expect(page.locator('debrief-title-bar')).toBeVisible();
    await expect(page.locator('debrief-inspector')).toBeVisible();
  });

  test('auto-hidden tool windows and pinned tabs survive a reload', async ({ page }) => {
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto hide', exact: true }).click();
    await (await panelMenu(page, 'Metrics')).getByRole('menuitem', { name: 'Pin tab' }).click();
    await expect
      .poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('debrief.layout') ?? 'null')?.extras))
      .toMatchObject({ autoHidden: [{ id: 'inspector', side: 'right' }], pinnedTabs: ['metrics'] });
    await page.reload();
    await expect(page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' })).toBeVisible();
    await expect(groupOf(page, 'Metrics').locator('.debrief-tab.pinned')).toHaveCount(1);
  });
});
