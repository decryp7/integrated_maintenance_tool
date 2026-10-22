import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, menubar, menuItem, openMenu, openMenuPath } from './menu-support';

// The VS-style menus of the in-app menu bar (Windows, Linux and the browser build) and the clean
// default layout. Also runs against the production build with the Tauri CSP.

const tabNames = async (page: Page) => (await page.getByRole('tab').locator('.imt-tab-label').allInnerTexts()).map((t) => t.trim()).sort();
/** The default (Investigate) perspective: the timeline and the registry viewer over the events. */
const CORE_TABS = ['Events', 'Registry', 'Timeline'];
/** The menu bar and its open menus (CDK overlays): what the menu checks scan, not the whole window (slow on WebKit). */
const OPEN_MENUS = 'imt-menu-bar, .cdk-overlay-container';
const inspectorTab = (page: Page) => page.locator('.imt-ah-tab[data-ah-id="inspector"]');

function watchProblems(page: Page): string[] {
  const problems: string[] = [];
  page.on('console', (msg) => {
    if (/Content Security Policy|Refused to/i.test(msg.text())) problems.push(msg.text());
  });
  page.on('pageerror', (err) => problems.push(`pageerror: ${err.message}`));
  return problems;
}

test.describe('menus', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
  });

  test('startup opens only the core views (the registry export opened is shown in its viewer)', async ({ page }) => {
    await expect.poll(() => tabNames(page)).toEqual(CORE_TABS);
    await expect(inspectorTab(page)).toBeVisible();
  });

  test('View ▸ Analysis ▸ Anomalies opens the anomalies view, and the View menu shows it checked', async ({ page }) => {
    const problems = watchProblems(page);
    const view = await openMenu(page, 'View');
    // Core tool windows at the top level, the rest in submenus.
    for (const name of ['Explorer', 'Fields', 'Inspector', 'Timeline', 'Events', 'Metrics', 'Home']) await expect(menuItem(view, name)).toBeVisible();
    for (const name of ['Analysis', 'Live', 'Profiling', 'Tools', 'Sharing', 'Diagnostics', 'Plugins']) await expect(view.getByRole('menuitem', { name, exact: true })).toHaveAttribute('aria-haspopup', 'menu');
    await expect(menuItem(view, 'Anomalies')).toHaveCount(0);
    await page.keyboard.press('Escape');
    await chooseMenu(page, 'View', 'Analysis', 'Anomalies');
    await expect(page.getByRole('tab', { name: 'Anomalies' })).toBeVisible();
    await expect(page.locator('imt-anomalies-view')).toBeVisible();
    const analysis = await openMenuPath(page, 'View', 'Analysis');
    await expect(menuItem(analysis, 'Anomalies')).toHaveAttribute('aria-checked', 'true');
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
    await expect(page.getByRole('menu')).toHaveCount(0);
    expect(problems).toEqual([]);
  });

  test('keyboard: arrows move through the menu bar and into submenus, type-ahead jumps, Escape closes one level', async ({ page }) => {
    // With a menu open, right moves to the next menu of the bar (File, Edit, View).
    // Keyboard only: the pointer stays out of the way (hovering the bar switches menus).
    await page.mouse.move(700, 600);
    await menubar(page).getByRole('menuitem', { name: 'File', exact: true }).focus();
    await page.keyboard.press('ArrowDown');
    await expect(page.getByRole('menu', { name: 'File' })).toBeVisible();
    await page.keyboard.press('ArrowRight');
    await expect(page.getByRole('menu', { name: 'Edit' })).toBeVisible();
    await page.keyboard.press('ArrowRight');
    const view = menubar(page).getByRole('menuitem', { name: 'View', exact: true });
    await expect(page.getByRole('menu', { name: 'View' })).toBeVisible();
    await expect(view).toHaveAttribute('aria-expanded', 'true');
    await expect(page.getByRole('menu', { name: 'Edit' })).toHaveCount(0);
    await expect(view).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(menuItem(page.getByRole('menu', { name: 'View' }), 'Explorer')).toBeFocused();
    // Type-ahead.
    await page.keyboard.press('l');
    const live = page.getByRole('menu', { name: 'View' }).getByRole('menuitem', { name: 'Live', exact: true });
    await expect(live).toBeFocused();
    await page.keyboard.press('ArrowRight');
    const sub = page.getByRole('menu', { name: 'Live' });
    await expect(sub).toBeVisible();
    // The live tail switch heads the Live submenu (F5), then its views.
    await expect(menuItem(sub, 'Toggle live tail')).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(menuItem(sub, 'Alerts')).toBeFocused();
    await page.keyboard.press('ArrowLeft');
    await expect(sub).toHaveCount(0);
    await expect(live).toBeFocused();
    await page.keyboard.press('ArrowUp');
    await page.keyboard.press('ArrowRight');
    const analysis = page.getByRole('menu', { name: 'Analysis' });
    await expect(analysis).toBeVisible();
    await expect(menuItem(analysis, 'Query results')).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await expect(menuItem(analysis, 'Anomalies')).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(analysis).toHaveCount(0);
    await expect(page.getByRole('menu', { name: 'View' })).toBeVisible();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(page.getByRole('menu')).toHaveCount(0);
    await expect(page.getByRole('tab', { name: 'Anomalies' })).toBeVisible();
  });

  test('menus stay on screen: long menus scroll, submenus open beside their item', async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 520 });
    const view = await openMenu(page, 'View');
    const box = (await view.boundingBox())!;
    expect(box.y + box.height).toBeLessThanOrEqual(520);
    const analyze = await openMenuPath(page, 'Analyze');
    const a = (await analyze.boundingBox())!;
    expect(a.y + a.height).toBeLessThanOrEqual(520);
    // Taller than the window: it scrolls instead of running off screen.
    expect(await analyze.evaluate((el) => getComputedStyle(el).overflowY)).toBe('auto');
    await page.keyboard.press('Escape');
    const help = await openMenu(page, 'Help');
    const h = (await help.boundingBox())!;
    expect(h.x + h.width).toBeLessThanOrEqual(1100);
    const plugins = await openMenuPath(page, 'View', 'Plugins');
    const p = (await plugins.boundingBox())!;
    expect(p.x + p.width).toBeLessThanOrEqual(1100);
    expect(p.y + p.height).toBeLessThanOrEqual(520);
  });

  test('View ▸ Other windows… finds any view by name, including those only in the palette', async ({ page }) => {
    await chooseMenu(page, 'View', 'Other windows…');
    const picker = page.getByRole('dialog', { name: 'Other windows' });
    const input = picker.getByRole('combobox', { name: 'Type a window name' });
    await expect(input).toBeFocused();
    await input.fill('callers');
    await expect(picker.getByRole('option')).toHaveCount(1);
    await expect(picker.getByRole('option').first()).toContainText('Callers and callees');
    await page.keyboard.press('Enter');
    await expect(picker).toHaveCount(0);
    await expect(page.getByRole('tab', { name: 'Callers and callees' })).toBeVisible();
  });

  test('Window ▸ Perspectives ▸ Reset perspective gives the clean default', async ({ page }) => {
    await chooseMenu(page, 'View', 'Analysis', 'Patterns');
    await chooseMenu(page, 'View', 'Plugins', 'Manage plugins');
    await chooseMenu(page, 'View', 'Inspector');
    await expect(page.getByRole('tab', { name: 'Patterns' })).toBeVisible();
    await expect(inspectorTab(page)).toHaveCount(0);
    await chooseMenu(page, 'Window', 'Perspectives', 'Reset perspective');
    await expect.poll(() => tabNames(page)).toEqual(CORE_TABS);
    await expect(inspectorTab(page)).toBeVisible();
  });

  test('Help has no dummy items, and keeps the app version in About', async ({ page }) => {
    const help = await openMenu(page, 'Help');
    await expect(help.getByRole('menuitem')).toContainText(['Welcome', 'Try the demo', 'Take the tour', 'Keyboard shortcuts']);
    await expect(menuItem(help, /IMT 0\.1\.0/)).toHaveCount(0);
    await menuItem(help, 'About IMT').click();
    await expect(page.getByRole('dialog', { name: 'About IMT' })).toContainText('Version');
  });

  test('open menus are accessible in both colour schemes', async ({ page }, testInfo) => {
    for (const theme of ['Dark', 'Light'] as const) {
      await chooseMenu(page, 'View', 'Theme', theme);
      await expect(page.locator('html')).toHaveAttribute('data-scheme', theme.toLowerCase());
      await openMenuPath(page, 'View', 'Analysis');
      await expectNoSeriousViolations(page, testInfo, `menus-view-${theme}`, OPEN_MENUS);
      await page.keyboard.press('Escape');
      await page.keyboard.press('Escape');
      await openMenuPath(page, 'Window', 'Perspectives');
      await expectNoSeriousViolations(page, testInfo, `menus-window-${theme}`, OPEN_MENUS);
      await page.keyboard.press('Escape');
      await page.keyboard.press('Escape');
    }
  });
});
