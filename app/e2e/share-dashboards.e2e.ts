import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

async function runCommand(page: Page, name: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(name);
  await expect(page.locator('imt-command-palette').getByRole('option').first()).toContainText(name);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

test.describe('dashboards', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('shows the profile dashboard, adds a tile and resizes one with the keyboard', async ({ page }, testInfo) => {
    await runCommand(page, 'Show dashboards');
    const view = page.locator('imt-dashboard-view');
    await expect(view.getByRole('combobox', { name: 'Dashboard' })).toHaveValue('orderservice-health');
    const errors = view.locator('[data-tile=errors]');
    await expect(errors.getByTestId('stat-value')).toHaveText(/\d/);
    await expect(view.locator('[data-tile=levels] imt-echart')).toBeVisible();
    await expect(view.locator('[data-tile=slow] table')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'dashboard', 'imt-dashboard-view');

    await view.getByRole('button', { name: 'Edit', exact: true }).click();
    await view.getByRole('button', { name: 'Add tile' }).click();
    const dialog = page.getByRole('dialog', { name: 'Add tile' });
    await dialog.getByLabel('Title', { exact: true }).fill('Fatal events');
    await dialog.getByLabel('Stats query', { exact: true }).fill('level = fatal | stats count() as fatal');
    await expectNoSeriousViolations(page, testInfo, 'add-tile', '[role=dialog]');
    await dialog.getByRole('button', { name: 'Add tile' }).click();
    await expect(dialog).toHaveCount(0);
    const added = view.locator('[data-tile=fatal-events]');
    await expect(added).toBeVisible();

    // Keyboard: focus a tile, Shift+ArrowRight makes it wider, ArrowDown moves it.
    await expect(errors).toHaveAttribute('data-size', '3x2');
    await errors.focus();
    await page.keyboard.press('Shift+ArrowRight');
    await expect(errors).toHaveAttribute('data-size', '4x2');
    await expect(view.getByTestId('dashboard-announce')).toContainText('4 by 2');
    await page.keyboard.press('ArrowDown');
    await expect(errors).toHaveAttribute('data-pos', '0,1');

    await view.getByRole('button', { name: 'Save' }).click();
    await expect(view.getByRole('button', { name: 'Edit', exact: true })).toBeVisible();
    await expect(view.locator('[data-tile=errors]')).toHaveAttribute('data-size', '4x2');
    await expect(view.locator('[data-tile=fatal-events]')).toBeVisible();
  });

  test('wall mode fills the window and Escape leaves it', async ({ page }) => {
    await runCommand(page, 'Show dashboards');
    const view = page.locator('imt-dashboard-view');
    await view.getByRole('button', { name: 'Wall mode' }).click();
    await expect(view).toHaveClass(/wall/);
    await view.getByRole('button', { name: 'Exit wall mode' }).click();
    await expect(view).not.toHaveClass(/wall/);
  });
});
