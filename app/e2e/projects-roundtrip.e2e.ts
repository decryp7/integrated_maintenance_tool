import { expect, test } from '@playwright/test';
import { closeStatusDetails, openStatusDetails } from './app';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand } from './tools-support';

test.describe('projects', () => {
  test('save, change, reopen: the query comes back and the unsaved dot follows', async ({ page }) => {
    await openAppWith(page);
    const title = page.getByTestId('project-title');
    await expect(title).toContainText('Untitled');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toHaveCount(0);

    const search = page.getByTestId('query-field');
    await search.fill('timed out');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toBeVisible();
    await runCommand(page, 'Save project as');
    await expect(page.getByRole('status').filter({ hasText: 'Saved Untitled' })).toBeVisible();
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toHaveCount(0);

    await search.fill('');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toBeVisible();
    // Unsaved changes: Open asks first, and Cancel keeps everything as it is.
    await runCommand(page, 'Open project');
    const unsaved = page.getByRole('dialog', { name: 'Unsaved changes' });
    await expect(unsaved).toContainText('Save the changes to Untitled before you open another project?');
    await expectNoSeriousViolations(page, test.info(), 'unsaved-changes', '[role=dialog]');
    await unsaved.getByRole('button', { name: 'Cancel' }).click();
    await expect(unsaved).toHaveCount(0);
    await expect(search).toHaveValue('');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toBeVisible();
    // Discard goes on and reopens the saved project.
    await runCommand(page, 'Open project');
    await unsaved.getByRole('button', { name: 'Discard' }).click();
    await expect(search).toHaveValue('timed out');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toHaveCount(0);

    await runCommand(page, 'Open recent project');
    const dialog = page.getByRole('dialog', { name: 'Recent projects' });
    await expect(dialog.getByRole('list', { name: 'Recent projects' }).getByRole('listitem')).toHaveCount(1);
    await expectNoSeriousViolations(page, test.info(), 'recent-projects', '[role=dialog]');
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();

    // A clean project starts a new one without asking.
    await runCommand(page, 'New project');
    // A new project is an empty workspace: the welcome page, without the filter bar.
    await expect(search).toHaveCount(0);
    await expect(title).toContainText('Untitled');
    await expect(unsaved).toHaveCount(0);
  });

  test('New project asks before discarding unsaved changes: Cancel keeps them, Discard proceeds', async ({ page }) => {
    await openAppWith(page);
    const search = page.getByTestId('query-field');
    await search.fill('timed out');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toBeVisible();
    const unsaved = page.getByRole('dialog', { name: 'Unsaved changes' });

    await runCommand(page, 'New project');
    for (const name of ['Save', 'Discard', 'Cancel']) await expect(unsaved.getByRole('button', { name, exact: true })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(unsaved).toHaveCount(0);
    await expect(search).toHaveValue('timed out');

    await runCommand(page, 'New project');
    await unsaved.getByRole('button', { name: 'Cancel' }).click();
    await expect(search).toHaveValue('timed out');
    const details = await openStatusDetails(page);
    await expect(details.locator('div', { hasText: /^Sources/ })).toContainText('6');
    await closeStatusDetails(page);

    await runCommand(page, 'New project');
    await unsaved.getByRole('button', { name: 'Discard' }).click();
    await expect(search).toHaveCount(0);
    await expect(page.locator('debrief-status-bar')).toContainText('No files open');
    await expect(page.getByRole('img', { name: 'Unsaved changes' })).toHaveCount(0);
  });

  test('Save in the unsaved-changes dialog saves first, then starts the new project', async ({ page }) => {
    await openAppWith(page);
    const search = page.getByTestId('query-field');
    await search.fill('timed out');
    await runCommand(page, 'New project');
    await page.getByRole('dialog', { name: 'Unsaved changes' }).getByRole('button', { name: 'Save' }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Saved Untitled' })).toBeVisible();
    await expect(search).toHaveCount(0);
    await runCommand(page, 'Open recent project');
    await expect(page.getByRole('dialog', { name: 'Recent projects' }).getByRole('listitem')).toHaveCount(1);
  });

  test('Mod+S saves the project', async ({ page }) => {
    await openAppWith(page);
    await page.keyboard.press('ControlOrMeta+s');
    await expect(page.getByRole('status').filter({ hasText: 'Saved' })).toBeVisible();
  });
});
