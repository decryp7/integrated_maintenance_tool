import { expect, Page, test } from '@playwright/test';
import { openAppWith, openView, runCommand } from './tools-support';
import { chooseMenu } from './menu-support';

/** The titles of the views offered (View ▸ Other windows… lists every one). */
async function viewMenu(page: Page): Promise<string[]> {
  await chooseMenu(page, 'View', 'Other windows…');
  const picker = page.getByRole('dialog', { name: 'Other windows' });
  // Plugin health is always offered: once it shows, the list is complete.
  await expect(picker.getByRole('option', { name: /Plugin health$/ })).toBeVisible();
  const items = await picker.locator('[role=option] .title').allInnerTexts();
  await page.keyboard.press('Escape');
  return items.map((t) => t.trim());
}

/** Whether the command palette lists a command. */
async function paletteOffers(page: Page, title: string): Promise<boolean> {
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill(title);
  const options = page.locator('imt-command-palette').getByRole('option');
  const offered = (await options.count()) > 0 && (await options.first().innerText()).includes(title);
  await page.keyboard.press('Escape');
  return offered;
}

test.describe('tools without the tools plugin', () => {
  test('safe mode leaves the tools out and the app works without them', async ({ page }) => {
    await openAppWith(page, '?mock-safe=1');
    await expect(page.getByTestId('safe-mode-banner')).toBeVisible();
    await expect.poll(() => viewMenu(page)).not.toContain('Performance Monitor');
    const menu = await viewMenu(page);
    expect(menu).not.toContain('Process Monitor');
    expect(menu).toContain('Plugin health');
    expect(await paletteOffers(page, 'Collect diagnostics bundle')).toBe(false);
    expect(await paletteOffers(page, 'Open Process Monitor')).toBe(false);
    // Core features still work.
    await openView(page, 'Plugin health');
    await expect(page.locator('imt-plugin-health-view')).toBeVisible();
    await expect(page.getByRole('alert').filter({ hasText: /safe mode|failed/i })).toHaveCount(0);
  });

  test('a normal start offers every tool', async ({ page }) => {
    await openAppWith(page);
    await expect.poll(() => viewMenu(page)).toContain('Performance Monitor');
    expect(await viewMenu(page)).toContain('Process Monitor');
    expect(await paletteOffers(page, 'Collect diagnostics bundle')).toBe(true);
  });

  test('a disabled tools plugin hides the tools too', async ({ page }) => {
    await openAppWith(page, '?mock-tools=disabled');
    await expect.poll(() => viewMenu(page)).not.toContain('Process Monitor');
    expect(await paletteOffers(page, 'Collect diagnostics bundle')).toBe(false);
  });
});

test.describe('tools of an unavailable plugin', () => {
  test('a quarantined tools plugin keeps its panels but says why and blocks the actions', async ({ page }) => {
    await openAppWith(page, '?mock-tools=quarantined');
    await openView(page, 'Process Monitor');
    const monitor = page.locator('imt-monitor-view');
    await expect(monitor.getByTestId('tool-unavailable')).toContainText('disabled after repeated crashes');
    await monitor.getByLabel('Target value 1').fill('OrderService*');
    await expect(monitor.getByRole('button', { name: 'Start' })).toBeDisabled();

    await openView(page, 'Performance Monitor');
    const perfmon = page.locator('imt-perfmon-view');
    await expect(perfmon.getByTestId('tool-unavailable')).toContainText('disabled after repeated crashes');
    await expect(perfmon.getByRole('button', { name: 'Generate XML' })).toBeDisabled();
    await expect(perfmon.getByRole('button', { name: /Install/ })).toHaveCount(0);

    await runCommand(page, 'Collect diagnostics bundle');
    const dialog = page.locator('imt-collector-dialog');
    await expect(dialog.getByTestId('tool-unavailable')).toContainText('repeated crashes');
    await expect(dialog.getByRole('button', { name: 'Collect', exact: true })).toBeDisabled();
  });

  test('an inactive tools plugin shows its dependency reason', async ({ page }) => {
    await openAppWith(page, '?mock-tools=inactive');
    await openView(page, 'Process Monitor');
    await expect(page.locator('imt-monitor-view').getByTestId('tool-unavailable')).toContainText('inactive: needs net.decryptology.windows');
  });
});
