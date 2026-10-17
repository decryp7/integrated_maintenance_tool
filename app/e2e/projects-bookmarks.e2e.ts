import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView, runCommand } from './tools-support';

test.describe('bookmarks, saved queries and highlight rules', () => {
  test('bookmarks the selected event with a note, jumps back, and keeps it in the project', async ({ page }, testInfo) => {
    await openAppWith(page);
    await page.locator('imt-events-grid [role=row].row').nth(2).click();
    await runCommand(page, 'Bookmark selected event');
    const panel = page.locator('imt-bookmarks-view');
    const list = panel.getByRole('list', { name: 'Bookmarks' });
    await expect(list.getByRole('listitem')).toHaveCount(1);
    await list.getByRole('textbox', { name: /^Note for / }).fill('start here');
    await list.getByRole('textbox', { name: /^Note for / }).press('Enter');
    await expectNoSeriousViolations(page, testInfo, 'bookmarks');

    await page.locator('imt-events-grid [role=row].row').nth(0).click();
    await list.getByRole('button', { name: /^Jump to bookmark/ }).click();
    await expect(page.locator('imt-events-grid .row.sel')).toBeVisible();

    await runCommand(page, 'Save project as');
    // Edit only once the save has written the bookmark.
    await expect(page.getByRole('status').filter({ hasText: /^Saved / })).toBeVisible();
    await list.getByRole('button', { name: /^Delete bookmark/ }).click();
    await expect(list).toHaveCount(0);
    await runCommand(page, 'Open project');
    // Deleting the bookmark left unsaved changes: discard them to reopen the saved project.
    await page.getByRole('dialog', { name: 'Unsaved changes' }).getByRole('button', { name: 'Discard' }).click();
    await expect(panel.getByRole('list', { name: 'Bookmarks' }).getByRole('textbox', { name: /^Note for / })).toHaveValue('start here');
  });

  test('saved queries re-apply the search', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Bookmarks');
    const search = page.getByRole('textbox', { name: 'Search messages and exceptions' });
    await search.fill('timed out');
    const panel = page.locator('imt-bookmarks-view');
    await panel.getByRole('textbox', { name: 'Saved query name' }).fill('Timeouts');
    await panel.getByRole('button', { name: 'Save current query' }).click();
    await search.fill('');
    await panel.getByRole('button', { name: 'Apply query Timeouts' }).click();
    await expect(search).toHaveValue('timed out');
  });

  test('highlight rules can be added and removed', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Bookmarks');
    const panel = page.locator('imt-bookmarks-view');
    await panel.getByLabel('Highlight field').fill('@level');
    await panel.getByLabel('Highlight condition').selectOption('eq');
    await panel.getByRole('textbox', { name: 'Highlight value' }).fill('Error');
    await panel.getByRole('radio', { name: 'Colour #e9ae3f' }).click();
    await panel.getByRole('button', { name: 'Add rule' }).click();
    const rules = panel.getByRole('list', { name: 'Highlight rules' });
    await expect(rules).toContainText('@level');
    await expect(rules).toContainText('Error');

    // The grid paints matching rows with a left colour bar (not only a tint), and only those rows.
    await page.getByRole('textbox', { name: 'Search messages and exceptions' }).fill('failed');
    const painted = page.locator('imt-events-grid [role=row].row.hl');
    await expect(painted.first()).toBeVisible();
    await expect(painted.first().locator('.lvl')).toContainText('ERR');
    const bar = await painted.first().evaluate((el) => {
      const s = getComputedStyle(el, '::before');
      return { width: s.width, color: s.backgroundColor };
    });
    expect(bar).toEqual({ width: '4px', color: 'rgb(233, 174, 63)' });
    await expect(page.locator('imt-events-grid [role=row].row:not(.hl) .lvl', { hasText: 'ERR' })).toHaveCount(0);

    await rules.getByRole('button', { name: 'Delete rule @level' }).click();
    await expect(rules).toHaveCount(0);
    await expect(painted).toHaveCount(0);
  });
});
