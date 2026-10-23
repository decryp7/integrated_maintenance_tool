import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { runCommand } from './analysis-support';
import { expectNoSeriousViolations } from './axe';

test.describe('grid multi-select', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('shift+click selects a range and "Infer pattern from lines…" uses every selected line', async ({ page }, testInfo) => {
    const grid = page.getByRole('grid', { name: 'Events' });
    await expect(grid).toHaveAttribute('aria-multiselectable', 'true');
    const rows = page.locator('debrief-events-grid [role=row].row');
    const messages = await Promise.all([0, 1, 2].map((i) => rows.nth(i).locator('.msg').innerText()));

    await rows.nth(0).click();
    await rows.nth(2).click({ modifiers: ['Shift'] });
    const selected = page.locator('debrief-events-grid [role=row][aria-selected=true]');
    await expect(selected).toHaveCount(3);
    await expectNoSeriousViolations(page, testInfo, 'grid-multi-select');

    await rows.nth(1).click({ button: 'right' });
    await expect(selected).toHaveCount(3);
    await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Infer pattern from lines…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Infer pattern from lines' });
    const lines = dialog.getByRole('list', { name: 'Which lines match' }).getByRole('listitem');
    await expect(lines).toHaveCount(3);
    for (const [i, m] of messages.entries()) await expect(lines.nth(i)).toContainText(m);
  });

  test('the palette command "Infer pattern from selected lines" uses the grid multi-selection', async ({ page }) => {
    const rows = page.locator('debrief-events-grid [role=row].row');
    const picked = [1, 2, 4];
    const messages = await Promise.all(picked.map((i) => rows.nth(i).locator('.msg').innerText()));

    await rows.nth(1).click();
    await rows.nth(2).click({ modifiers: ['Shift'] });
    await rows.nth(4).click({ modifiers: ['ControlOrMeta'] });
    await expect(page.locator('debrief-events-grid [role=row][aria-selected=true]')).toHaveCount(3);

    await runCommand(page, 'Infer pattern from selected lines');
    const dialog = page.getByRole('dialog', { name: 'Infer pattern from lines' });
    const lines = dialog.getByRole('list', { name: 'Which lines match' }).getByRole('listitem');
    await expect(lines).toHaveCount(3);
    for (const [i, m] of messages.entries()) await expect(lines.nth(i)).toContainText(m);
  });

  test('ctrl/cmd+click toggles rows and shift+arrow extends; a plain click selects one row again', async ({ page }) => {
    const rows = page.locator('debrief-events-grid [role=row].row');
    const selected = page.locator('debrief-events-grid [role=row][aria-selected=true]');

    await rows.nth(0).click();
    await rows.nth(3).click({ modifiers: ['ControlOrMeta'] });
    await expect(selected).toHaveCount(2);
    await rows.nth(3).click({ modifiers: ['ControlOrMeta'] });
    await expect(selected).toHaveCount(1);

    await rows.nth(4).click();
    await page.keyboard.press('Shift+ArrowDown');
    await page.keyboard.press('Shift+ArrowDown');
    await expect(selected).toHaveCount(3);
    await expect(page.locator('debrief-inspector .message')).toContainText(await rows.nth(6).locator('.msg').innerText());

    await rows.nth(1).click();
    await expect(selected).toHaveCount(1);
  });
});
