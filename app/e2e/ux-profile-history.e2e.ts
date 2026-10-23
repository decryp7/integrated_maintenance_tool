import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openMenu, showProfileEditor } from './menu-support';

async function openProfile(page: Page) {
  await openApp(page);
  await showProfileEditor(page);
  await expect(page.locator('debrief-profile-editor .bar .name')).toHaveText(/\(Serilog\)/);
  await expect(page.locator('debrief-profile-editor .bar .good')).toBeVisible();
}

test.describe('profile history', () => {
  test('restores an earlier version, applied and undoable', async ({ page }, testInfo) => {
    await openProfile(page);
    const editor = page.locator('debrief-profile-editor');
    await editor.getByRole('tab', { name: 'History', exact: true }).click();
    await expect(editor).toContainText('No earlier versions yet');

    // Save a change: the version it replaces appears in the history.
    await editor.getByRole('tab', { name: 'Form', exact: true }).click();
    await page.getByRole('navigation', { name: 'Profile sections' }).getByRole('button', { name: /^General/ }).click();
    await page.getByLabel('Profile name').fill('Renamed by e2e');
    await editor.getByRole('button', { name: 'Save & apply' }).click();
    await expect(editor.locator('.bar .name')).toHaveText('Renamed by e2e');

    await editor.getByRole('tab', { name: 'History', exact: true }).click();
    const versions = page.getByRole('list', { name: 'Saved versions' }).getByRole('button');
    await expect(versions).toHaveCount(1);
    await versions.first().click();
    await expect(editor.locator('.summary')).toContainText('Restoring it would remove');
    await expect(editor.locator('.diff .l.added').filter({ hasText: '(Serilog)' }).first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'profile-history', 'debrief-profile-history');

    await editor.getByRole('button', { name: 'Restore this version' }).click();
    await expect(editor.locator('.bar .name')).toHaveText(/\(Serilog\)/);
    await expect(editor.locator('.summary')).toContainText('same as the current profile');

    await openMenu(page, 'Edit');
    await page.getByRole('menuitem', { name: /^Undo profile restore/ }).click();
    await expect(editor.locator('.bar .name')).toHaveText('Renamed by e2e');
  });
});
