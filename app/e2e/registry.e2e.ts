import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, menuItem, openMenuPath } from './menu-support';

test.describe('registry viewer plugin', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('shows the registry as a tree and selecting a value shows it in the inspector', async ({ page }, testInfo) => {
    await page.getByRole('tab', { name: 'Registry', exact: true }).click();
    const tree = page.getByRole('tree', { name: 'Registry' });
    await expect(tree).toBeVisible();
    await expect(tree.getByRole('treeitem').first()).toContainText('HKEY_CURRENT_USER');
    const timeout = tree.getByRole('treeitem', { name: /TimeoutSeconds/ });
    await expect(timeout).toContainText('REG_DWORD');
    await expect(timeout).toContainText('30');
    await timeout.click();
    await expect(timeout).toHaveAttribute('aria-selected', 'true');
    const inspector = page.locator('imt-inspector');
    await expect(inspector.locator('.prop').filter({ hasText: 'ValueName' })).toContainText('TimeoutSeconds');
    await expect(inspector.locator('.prop').filter({ hasText: 'ValueType' })).toContainText('REG_DWORD');
    await expectNoSeriousViolations(page, testInfo, 'registry');
  });

  test('keyboard navigation expands, collapses and filters', async ({ page }) => {
    await page.getByRole('tab', { name: 'Registry', exact: true }).click();
    const tree = page.getByRole('tree', { name: 'Registry' });
    const first = tree.getByRole('treeitem').first();
    await first.focus();
    await expect(first).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('ArrowLeft');
    await expect(first).toHaveAttribute('aria-expanded', 'false');
    await page.keyboard.press('ArrowRight');
    await expect(first).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('ArrowDown');
    await expect(tree.getByRole('treeitem').nth(1)).toBeFocused();

    await page.getByRole('searchbox', { name: 'Filter registry' }).fill('MaxFileSize');
    await expect(tree.getByRole('treeitem', { name: /MaxFileSize/ })).toContainText('104857600');
    await expect(tree.getByRole('treeitem', { name: /TimeoutSeconds/ })).toHaveCount(0);
  });

  test('the registry panel can be reopened from the View menu and from the palette', async ({ page }) => {
    const tab = page.getByRole('tab', { name: 'Registry', exact: true });
    await tab.focus();
    await page.keyboard.press('Delete');
    await expect(tab).toHaveCount(0);
    await chooseMenu(page, 'View', 'Plugins', 'Registry');
    await expect(tab).toHaveAttribute('aria-selected', 'true');
    await tab.focus();
    await page.keyboard.press('Delete');
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('registry viewer');
    await page.keyboard.press('Enter');
    await expect(tab).toHaveAttribute('aria-selected', 'true');
  });

  test('is only offered while a registry source is open', async ({ page }) => {
    const sidebar = page.locator('imt-sidebar');
    await sidebar.locator('.src', { hasText: 'registry-export.reg' }).hover();
    await sidebar.getByRole('button', { name: 'Remove registry-export.reg' }).click();
    await expect(page.getByRole('tab', { name: 'Registry', exact: true })).toHaveCount(0);
    const plugins = await openMenuPath(page, 'View', 'Plugins');
    await expect(menuItem(plugins, 'Manage plugins')).toBeVisible();
    await expect(menuItem(plugins, 'Registry')).toHaveCount(0);
  });
});
