import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, menuItem } from './menu-support';

test.describe('source files and colours', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('a row menu opens the event file at its line or shows it in its folder', async ({ page }, testInfo) => {
    await page.locator('debrief-events-grid [role=row].row').first().click({ button: 'right' });
    const menu = page.getByRole('menu', { name: 'Actions' });
    await expect(menuItem(menu, 'Open file')).toBeEnabled();
    await expect(menuItem(menu, /^Open at line \d+$/)).toBeEnabled();
    await expect(menuItem(menu, 'Show in folder')).toBeEnabled();
    await expectNoSeriousViolations(page, testInfo, 'row-menu-files');
    await menuItem(menu, /^Open at line \d+$/).click();
    await expect(menu).toHaveCount(0);
    // The mock "opens" in the editor: no error and no fallback notice.
    await expect(page.getByText(/no longer open|VS Code was not found/)).toHaveCount(0);
  });

  test('the Explorer row has the file actions and the file colour', async ({ page }) => {
    const row = page.locator('debrief-sidebar .src').first();
    await expect(row.locator('debrief-source-swatch')).toHaveAttribute('title', /^File: /);
    await row.hover();
    await row.getByRole('button', { name: /^Actions for / }).click();
    const menu = page.getByRole('menu', { name: 'Actions' });
    await expect(menuItem(menu, 'Show in folder')).toBeVisible();
    await expect(menuItem(menu, 'Open file')).toBeVisible();
    await page.keyboard.press('Escape');
  });

  test('rows, lanes and the inspector carry the colour, and View › Source colours switches it', async ({ page }, testInfo) => {
    const stripes = page.locator('debrief-events-grid [data-testid=source-stripe]');
    await expect(stripes.first()).toHaveAttribute('aria-label', /^File: /);
    await expect(page.locator('debrief-timeline-panel .lane-label debrief-source-swatch').first()).toBeVisible();
    await page.locator('debrief-events-grid [role=row].row').first().click();
    await expect(page.locator('debrief-inspector .src-line debrief-source-swatch')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'source-colours');

    await chooseMenu(page, 'View', 'Source colours', 'By instance');
    await expect(stripes.first()).toHaveAttribute('aria-label', /^Instance: /);
    await expect(page.locator('debrief-sidebar debrief-source-swatch')).toHaveCount(0);

    await chooseMenu(page, 'View', 'Source colours', 'Off');
    await expect(stripes).toHaveCount(0);
    await expect(page.locator('debrief-timeline-panel .lane-label debrief-source-swatch')).toHaveCount(0);
    // The choice is a user preference: it survives a reload.
    await openApp(page);
    await expect(page.locator('debrief-events-grid [role=row].row').first()).toBeVisible();
    await expect(stripes).toHaveCount(0);
  });
});
