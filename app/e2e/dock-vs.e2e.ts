import { expect, Locator, Page, test } from '@playwright/test';
import { ensureInspector, openApp, openView } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';
import { showProfileEditor } from './menu-support';

// Visual Studio-style docking: drop guides, auto hide, the panel menu, pinned tabs, the Ctrl+Tab navigator and
// named window layouts.

const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const groups = (page: Page) => page.locator('.dv-groupview');
const groupOf = (page: Page, name: string) => groups(page).filter({ has: tab(page, name) });
const SCREENSHOTS = '../docs/screenshots';

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
  // These start from the fuller arrangement (the inspector docked, Metrics tabbed with the timeline) that the
  // minimal default no longer opens.
  // The docking mechanics are checked on the Full layout (timeline, query bar and events in three groups).
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('imt.layout.mode.v1', 'full'));
    await openApp(page);
    await ensureInspector(page);
    await openView(page, 'metrics');
  });

  test('dragging a tab onto the left edge guide docks it along the whole left edge', async ({ page, browserName }) => {
    const dock = await box(page, '.imt-dock-mount');
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
    await expect(page.locator('.imt-guide-left.hot')).toHaveCount(1);
    await expect(page.locator('.imt-guide-edges .imt-guide')).toHaveCount(4);
    if (browserName === 'chromium') await page.screenshot({ path: `${SCREENSHOTS}/docking-guides-edge.png` });
    await page.mouse.up();
    const metrics = await box(page, 'imt-metrics-view');
    const events = await box(page, 'imt-events-grid');
    expect(metrics.x).toBeLessThan(events.x);
    // Full height: from above the timeline to below the events.
    expect(metrics.height).toBeGreaterThan(events.height + 200);
  });

  test('dragging a tab onto the centre of the compass tabs it into that group', async ({ page, browserName }) => {
    const content = await box(page, '.dv-groupview:has(imt-events-grid) .dv-content-container');
    await dragTabTo(page, 'Metrics', content.x + content.width / 2, content.y + content.height / 2);
    await expect(page.locator('.imt-guide-center.hot')).toHaveCount(1);
    if (browserName === 'chromium') await page.screenshot({ path: `${SCREENSHOTS}/docking-guides-compass.png` });
    await page.mouse.up();
    await expect(groupOf(page, 'Events')).toContainText('Metrics');
    await expect(groups(page)).toHaveCount(4);
  });

  test('a released drag away from any guide is cancelled: the panel stays where it was and nothing floats', async ({ page }) => {
    const before = await groupOf(page, 'Metrics').locator('[role="tab"]').allInnerTexts();
    const content = await box(page, '.dv-groupview:has(imt-events-grid) .dv-content-container');
    await dragTabTo(page, 'Metrics', content.x + 40, content.y + content.height - 20);
    await expect(page.locator('.imt-float-cue')).toHaveText('Release to cancel');
    await page.mouse.up();
    await expect(page.locator('.dv-resize-container')).toHaveCount(0);
    await expect(groupOf(page, 'Events')).not.toContainText('Metrics');
    expect(await groupOf(page, 'Metrics').locator('[role="tab"]').allInnerTexts()).toEqual(before);
  });

  test('auto hide: the inspector collapses to a side bar, slides out on hover, hides on Escape, pins back', async ({ page, browserName }) => {
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto hide', exact: true }).click();
    await expect(tab(page, 'Inspector')).toHaveCount(0);
    const sideTab = page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' });
    await expect(sideTab).toBeVisible();
    await expect(groups(page)).toHaveCount(3);

    await sideTab.hover();
    const flyout = page.getByRole('region', { name: 'Inspector' });
    await expect(flyout).toBeVisible();
    await expect(flyout.locator('imt-inspector')).toBeVisible();
    await expect(sideTab).toHaveAttribute('aria-expanded', 'true');
    await flyout.evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));
    if (browserName === 'chromium') await page.screenshot({ path: `${SCREENSHOTS}/docking-autohide.png` });
    await expectNoSeriousViolations(page, test.info(), 'auto-hide');
    await flyout.locator('imt-inspector').click();
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
    await expect(groups(page)).toHaveCount(4);
    await expect(page.locator('imt-inspector')).toBeVisible();
    // Back to the right of the events grid once the dock layout has settled.
    await expect
      .poll(async () => {
        const inspector = await box(page, 'imt-inspector');
        const events = await box(page, 'imt-events-grid');
        return inspector.x - (events.x + events.width);
      })
      .toBeGreaterThanOrEqual(-2);
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
    const tabs = groupOf(page, 'Timeline').getByRole('tab');
    await expect(tabs.first()).toHaveAccessibleName('Profile');
    await expect(groupOf(page, 'Timeline').locator('.imt-tab.pinned')).toHaveCount(1);
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

  test('a named window layout is saved and applied with Ctrl+Alt+1', async ({ page }) => {
    await runCommand(page, 'Save window layout…');
    const name = page.getByRole('textbox', { name: 'Layout name' });
    await name.fill('Triage');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(name).toHaveCount(0);
    await tab(page, 'Metrics').focus();
    await page.keyboard.press('Delete');
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto hide', exact: true }).click();
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    await page.keyboard.press('Control+Alt+1');
    await expect(tab(page, 'Metrics')).toBeVisible();
    await expect(tab(page, 'Inspector')).toBeVisible();
    await expect(page.getByRole('navigation', { name: /Auto-hidden tool windows/ })).toHaveCount(0);
    // Saved app-wide, separately from the project.
    expect(await page.evaluate(() => JSON.parse(localStorage.getItem('imt.windowLayouts') ?? 'null')?.layouts?.[0]?.name)).toBe('Triage');
    await runCommand(page, 'Manage window layouts…');
    await expect(page.getByRole('list', { name: 'Saved window layouts' })).toContainText('Triage');
    await expectNoSeriousViolations(page, test.info(), 'manage-layouts');
  });

  test('Reset window layout restores the minimal default arrangement', async ({ page }) => {
    await (await panelMenu(page, 'Metrics')).getByRole('menuitem', { name: 'Pin tab' }).click();
    await runCommand(page, 'Reset window layout');
    // Timeline, query bar and events; the inspector auto-hidden on the right; Metrics closed.
    await expect(tab(page, 'Inspector')).toHaveCount(0);
    await expect(page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' })).toBeVisible();
    await expect(tab(page, 'Metrics')).toHaveCount(0);
    await expect(page.locator('.imt-tab.pinned')).toHaveCount(0);
    await expect(groups(page)).toHaveCount(3);
  });

  test('full screen hides the chrome and the tool windows; again restores them', async ({ page }) => {
    await page.keyboard.press('Shift+Alt+Enter');
    await expect(page.locator('imt-title-bar')).toBeHidden();
    await expect(page.locator('imt-inspector')).toBeHidden();
    await page.keyboard.press('Shift+Alt+Enter');
    await expect(page.locator('imt-title-bar')).toBeVisible();
    await expect(page.locator('imt-inspector')).toBeVisible();
  });

  test('auto-hidden tool windows and pinned tabs survive a reload', async ({ page }) => {
    await groupOf(page, 'Inspector').getByRole('button', { name: 'Panel menu', exact: true }).click();
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Auto hide', exact: true }).click();
    await (await panelMenu(page, 'Metrics')).getByRole('menuitem', { name: 'Pin tab' }).click();
    await expect
      .poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('imt.layout') ?? 'null')?.extras))
      .toMatchObject({ autoHidden: [{ id: 'inspector', side: 'right' }], pinnedTabs: ['metrics'] });
    await page.reload();
    await expect(page.getByRole('navigation', { name: 'Auto-hidden tool windows (right)' }).getByRole('button', { name: 'Inspector' })).toBeVisible();
    await expect(groupOf(page, 'Timeline').locator('.imt-tab.pinned')).toHaveCount(1);
  });
});
