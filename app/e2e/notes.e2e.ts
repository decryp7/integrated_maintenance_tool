import { expect, test } from '@playwright/test';
import { runCommand } from './analysis-support';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// Notes (ADR 0021) against the browser build's mock notes backend (src/app/notes/mock/mock-notes-backend.ts): a note
// on an event row shows a glyph on the row and is listed in the Notes panel, which jumps back to the event; an area
// marked on an image of the workspace gets a note and is drawn as a region.

test('a note on an event row is marked on the row, listed and jumped to', async ({ page }, testInfo) => {
  test.slow();
  await openAppWith(page, '');
  const row = page.locator('debrief-events-grid [role=row].row').nth(2);
  await expect(row).toBeVisible();
  await row.click({ button: 'right' });
  await page.getByRole('menu', { name: 'Actions' }).getByRole('menuitem', { name: 'Add note…' }).click();

  const editor = page.getByRole('dialog', { name: 'Add a note' });
  await editor.getByTestId('note-text').fill('**Root cause**: the disk filled up');
  await editor.getByTestId('note-tags').fill('disk');
  await editor.getByTestId('note-preview-tab').click();
  await expect(editor.getByTestId('note-preview').locator('strong')).toHaveText('Root cause');
  await expectNoSeriousViolations(page, testInfo, 'note-editor', '[role=dialog]');
  await editor.getByTestId('note-save').click();
  await expect(editor).toHaveCount(0);
  await expect(row.getByTestId('note-glyph')).toHaveAttribute('aria-label', 'Root cause: the disk filled up');

  await runCommand(page, 'Show notes');
  const panel = page.locator('debrief-notes-panel');
  const item = panel.getByTestId('note-item');
  await expect(item).toHaveCount(1);
  await expect(item).toContainText('#disk');
  await expectNoSeriousViolations(page, testInfo, 'notes-panel', 'debrief-notes-panel');
  // Scroll the grid away, then jump back to the noted event.
  await page.locator('debrief-events-grid .viewport').evaluate((v) => (v.scrollTop = v.scrollHeight));
  await item.getByTestId('note-jump').click();
  await expect(page.locator('debrief-events-grid .row.sel').getByTestId('note-glyph')).toBeVisible();
});

test('an area marked on an image gets a note and is drawn as a region', async ({ page }) => {
  test.slow();
  await openAppWith(page, `?mockOpen=${encodeURIComponent('/shots/error-dialog.png')}`);
  const sidebar = page.locator('debrief-sidebar');
  await sidebar.getByRole('button', { name: 'Open files' }).click();
  await sidebar.getByRole('button', { name: 'Mark up image error-dialog.png' }).click();

  const view = page.locator('debrief-note-image-view');
  await expect(view.getByTestId('note-image-img')).toHaveAttribute('src', /^data:image\//);
  const stage = view.getByTestId('note-image-stage');
  const box = (await stage.boundingBox())!;
  await page.mouse.move(box.x + 30, box.y + 30);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + 80, { steps: 5 });
  await page.mouse.up();
  await expect(view.getByTestId('note-region-pending')).toHaveCount(1);
  await view.getByTestId('note-image-add').click();

  const editor = page.getByRole('dialog', { name: 'Add a note' });
  await expect(editor.getByTestId('note-anchor')).toContainText('error-dialog.png');
  await editor.getByTestId('note-text').fill('The red banner names the failing service');
  await editor.getByTestId('note-save').click();
  await expect(editor).toHaveCount(0);

  const region = view.getByTestId('note-region');
  await expect(region).toHaveCount(1);
  await expect(region).toHaveAttribute('aria-label', 'The red banner names the failing service');
  await expect(region).toHaveClass(/focused/);
  await expect(view.getByTestId('note-image-note')).toContainText('red banner');
});
