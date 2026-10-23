import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';
import { chooseMenu, menuItem, openMenuPath } from './menu-support';

// Perspectives: named window arrangements switched from the activity bar, Window ▸ Perspectives and Ctrl+1…9.

const bar = (page: Page) => page.locator('debrief-activity-bar');
const switcher = (page: Page) => bar(page).locator('[data-activity-id=perspectives]');
const flyout = (page: Page) => page.getByRole('menu', { name: 'Perspectives' });
const tab = (page: Page, name: string) => page.getByRole('tab', { name, exact: true });
const rowByName = (page: Page, name: string) => page.getByTestId('perspective-row').filter({ hasText: name });

async function openFlyout(page: Page) {
  await switcher(page).click();
  await expect(flyout(page)).toBeVisible();
  return flyout(page);
}

test.describe('perspectives', () => {
  test.beforeEach(async ({ page }) => {
    // Cleared once, not on the reload the remember-across-reload test makes.
    await page.addInitScript(() => {
      if (sessionStorage.getItem('debrief-e2e-cleared')) return;
      sessionStorage.setItem('debrief-e2e-cleared', '1');
      localStorage.clear();
    });
    await openApp(page);
  });

  test('the activity-bar switcher lists the built-in perspectives, Investigate checked, with Save and Manage', async ({ page }, testInfo) => {
    const menu = await openFlyout(page);
    for (const name of ['Investigate', 'Profiling', 'Live', 'Diagnostics']) await expect(menu.getByRole('menuitemcheckbox', { name })).toBeVisible();
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Investigate' })).toHaveAttribute('aria-checked', 'true');
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Profiling' })).toHaveAttribute('aria-checked', 'false');
    await expect(menu.getByRole('menuitemcheckbox', { name: 'Investigate' })).toHaveAttribute('aria-keyshortcuts', /1/);
    await expect(menu.getByRole('menuitem', { name: 'Save current as perspective…' })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: 'Manage perspectives…' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'perspectives-flyout', '.cdk-overlay-container');
    await menu.getByRole('menuitemcheckbox', { name: 'Diagnostics' }).click();
    await expect(flyout(page)).toHaveCount(0);
    // Diagnostics adds problems, jobs, ... below the events; the timeline and events stay.
    await expect(page.getByRole('tab', { name: /^Problems/ })).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(page.locator('debrief-timeline-panel')).toBeVisible();
  });

  test('Window ▸ Perspectives lists them with their keys and switches', async ({ page }, testInfo) => {
    const menu = await openMenuPath(page, 'Window', 'Perspectives');
    for (const name of ['Perspective: Investigate', 'Perspective: Profiling', 'Perspective: Live', 'Perspective: Diagnostics', 'Save current as perspective…', 'Manage perspectives…', 'Reset perspective'])
      await expect(menuItem(menu, name)).toBeVisible();
    await expect(menuItem(menu, 'Perspective: Investigate')).toHaveAttribute('aria-checked', 'true');
    await expectNoSeriousViolations(page, testInfo, 'perspectives-window-menu', '.cdk-overlay-container, debrief-menu-bar');
    await menuItem(menu, 'Perspective: Live').click();
    await expect(page.locator('.debrief-tab[data-panel-id="tail-status"]')).toBeVisible();
    await chooseMenu(page, 'Window', 'Perspectives', 'Perspective: Investigate');
    await expect(page.locator('.debrief-tab[data-panel-id="tail-status"]')).toHaveCount(0);
  });

  test('Ctrl+2 switches to Profiling and Ctrl+1 restores Investigate as it was left', async ({ page }) => {
    await runCommand(page, 'Show anomalies');
    await expect(tab(page, 'Anomalies')).toBeVisible();
    await page.keyboard.press('ControlOrMeta+2');
    await expect(page.locator('debrief-captures-view')).toBeVisible();
    await expect(page.getByTestId('captures-landing')).toBeVisible();
    await expect(page.locator('debrief-timeline-panel')).toHaveCount(0);
    await expect(page.getByRole('grid', { name: 'Events' })).toHaveCount(0);

    await page.keyboard.press('ControlOrMeta+1');
    await expect(page.locator('debrief-timeline-panel')).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();
    await expect(tab(page, 'Anomalies')).toBeVisible();
    // The choice is remembered across a reload.
    await page.keyboard.press('ControlOrMeta+2');
    await expect(page.locator('debrief-captures-view')).toBeVisible();
    // The layout is saved a moment after a change: wait for it before reloading.
    await expect.poll(() => page.evaluate(() => Object.keys(JSON.parse(localStorage.getItem('debrief.layout') ?? '{}').layout?.panels ?? {}).includes('profiling-captures'))).toBe(true);
    await page.reload();
    await expect(page.getByRole('grid', { name: 'Events' })).toHaveCount(0);
    await expect(tab(page, 'Captures')).toBeVisible({ timeout: 20_000 });
    await switcher(page).click();
    await expect(flyout(page).getByRole('menuitemcheckbox', { name: 'Profiling' })).toHaveAttribute('aria-checked', 'true');
  });

  test('save the current arrangement as a perspective, rename and delete it in Manage perspectives', async ({ page }, testInfo) => {
    await runCommand(page, 'Show anomalies');
    const menu = await openFlyout(page);
    await menu.getByRole('menuitem', { name: 'Save current as perspective…' }).click();
    const save = page.getByRole('dialog', { name: 'Save as perspective' });
    await expect(save).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'perspective-save', '[role=dialog]');
    const name = save.getByLabel('Perspective name');
    await name.fill('Triage');
    await save.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(save).toHaveCount(0);
    // It is the fifth perspective (Ctrl+5) and the active one.
    await switcher(page).click();
    await expect(flyout(page).getByRole('menuitemcheckbox', { name: 'Triage' })).toHaveAttribute('aria-checked', 'true');
    await page.keyboard.press('Escape');

    await chooseMenu(page, 'Window', 'Perspectives', 'Manage perspectives…');
    const manage = page.getByRole('dialog', { name: 'Perspectives' });
    await expect(manage.getByTestId('perspective-row')).toHaveCount(5);
    await expectNoSeriousViolations(page, testInfo, 'perspective-manage', '[role=dialog]');
    await manage.getByRole('button', { name: 'Rename Triage' }).click();
    const rename = manage.getByRole('textbox', { name: 'New name for Triage' });
    await rename.fill('Outage');
    await rename.press('Enter');
    await expect(rowByName(page, 'Outage')).toBeVisible();
    await expect(rowByName(page, 'Triage')).toHaveCount(0);
    await expect(manage.getByRole('button', { name: 'Switch to Profiling' })).toBeEnabled();
    await manage.getByRole('button', { name: 'Delete Outage' }).click();
    await expect(manage.getByTestId('perspective-row')).toHaveCount(4);
    await expect(rowByName(page, 'Outage')).toHaveCount(0);
    await manage.getByRole('button', { name: 'Switch to Profiling' }).click();
    await expect(manage).toHaveCount(0);
    await expect(page.locator('debrief-captures-view')).toBeVisible();
  });

  test('Reset perspective rebuilds the active one as it comes', async ({ page }) => {
    await runCommand(page, 'Show anomalies');
    await runCommand(page, 'Show patterns');
    await expect(tab(page, 'Patterns')).toBeVisible();
    await chooseMenu(page, 'Window', 'Perspectives', 'Reset perspective');
    await expect(tab(page, 'Anomalies')).toHaveCount(0);
    await expect(tab(page, 'Patterns')).toHaveCount(0);
    await expect(tab(page, 'Timeline')).toBeVisible();
    await expect(tab(page, 'Events')).toBeVisible();
  });

  test('Ctrl+Alt+1 focuses the first dock group', async ({ page }) => {
    await page.getByRole('grid', { name: 'Events' }).focus();
    await page.keyboard.press('Control+Alt+Digit1');
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest('.dv-groupview') !== null && document.activeElement?.closest('.dv-groupview') === document.querySelectorAll('.dv-groupview')[0])).toBe(true);
    await page.keyboard.press('Control+Alt+Digit3');
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest('.dv-groupview') === document.querySelectorAll('.dv-groupview')[2])).toBe(true);
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`the switcher fly-out is accessible (${scheme})`, async ({ page }, testInfo) => {
      await page.emulateMedia({ colorScheme: scheme });
      await openFlyout(page);
      await expectNoSeriousViolations(page, testInfo, `perspectives-${scheme}`, '.cdk-overlay-container');
    });
  }
});
