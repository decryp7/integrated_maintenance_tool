import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { openMenu } from './menu-support';

async function openEditMenu(page: Page) {
  return openMenu(page, 'Edit');
}

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  await expect(page.getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

test.describe('undo and redo', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('a filter change: keyboard and Edit menu, with the label of the edit', async ({ page }, testInfo) => {
    const levels = page.getByRole('toolbar', { name: 'Levels' });
    const err = levels.getByRole('button', { name: /^ERR/ });
    const all = await initialTotal(page);
    const errors = parseCount(await err.locator('.faint').innerText());

    await err.click();
    await expect(gridTotal(page)).toHaveText(String(errors));

    // Keyboard (focus is on the chip, not in a text field).
    await page.keyboard.press('ControlOrMeta+z');
    await expect(err).toHaveAttribute('aria-pressed', 'false');
    await expect(gridTotal(page)).toHaveText(all);
    await page.keyboard.press('ControlOrMeta+Shift+z');
    await expect(err).toHaveAttribute('aria-pressed', 'true');
    await expect(gridTotal(page)).toHaveText(String(errors));

    // Edit menu names what would be undone.
    const menu = await openEditMenu(page);
    const undoItem = menu.getByRole('menuitem', { name: /^Undo level filter change/ });
    await expect(undoItem).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'edit-menu');
    await undoItem.click();
    await expect(gridTotal(page)).toHaveText(all);
    await openEditMenu(page);
    await expect(page.getByRole('menuitem', { name: /^Undo \(nothing to undo\)/ })).toBeVisible();
    await page.getByRole('menuitem', { name: /^Redo level filter change/ }).click();
    await expect(err).toHaveAttribute('aria-pressed', 'true');
  });

  test('typing in the search box keeps the browser undo; outside it Mod+Z undoes the search as one edit', async ({ page }) => {
    const all = await initialTotal(page);
    const search = page.getByRole('textbox', { name: 'Search messages and exceptions' });
    await search.pressSequentially('timed out', { delay: 20 });
    await expect(gridTotal(page)).not.toHaveText(all);
    await page.getByRole('toolbar', { name: 'Levels' }).getByRole('button', { name: /^ERR/ }).focus();
    await page.keyboard.press('ControlOrMeta+z');
    await expect(search).toHaveValue('');
    await expect(gridTotal(page)).toHaveText(all);
  });

  test('a marker rule edit is undone and redone in the engine', async ({ page }) => {
    await runCommand(page, 'Manage markers');
    const table = page.getByRole('table', { name: 'Marker rules' });
    await expect(table).toBeVisible();
    const row = table.locator('tr[data-kind=warning]');
    await row.getByRole('checkbox', { name: /^Disable/ }).click();
    await expect(row.getByRole('checkbox', { name: /^Enable/ })).toBeVisible();

    const menu = await openEditMenu(page);
    await expect(menu.getByRole('menuitem', { name: /^Undo marker rule edit/ })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);

    // The rule list is reloaded from the engine's profile after each save, so it shows the engine's state.
    await page.keyboard.press('ControlOrMeta+z');
    await expect(row.getByRole('checkbox', { name: /^Disable/ })).toBeVisible();
    await expect(page.getByRole('status').filter({ hasText: 'Undone: marker rule edit' })).toBeVisible();
    await page.keyboard.press('ControlOrMeta+Shift+z');
    await expect(row.getByRole('checkbox', { name: /^Enable/ })).toBeVisible();
  });
});
