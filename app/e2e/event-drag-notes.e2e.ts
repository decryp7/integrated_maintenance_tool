import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';

// Dragging an events-grid row onto the Notes panel opens the note editor on that event. Browsers cannot start a real
// OS drag from automation, so the drag events are dispatched with a shared DataTransfer (what a drag carries).

const rows = (page: Page) => page.locator('imt-events-grid [role=row].row');

async function openNotes(page: Page) {
  await runCommand(page, 'Show notes');
  const panel = page.locator('imt-notes-panel');
  await expect(panel).toBeVisible();
  return panel;
}

/** Drags row `index` of the grid onto the Notes panel: dragstart on the row, then dragover and drop on the panel. */
async function dragRowToNotes(page: Page, index: number): Promise<void> {
  const handle = await rows(page).nth(index).elementHandle();
  const target = await page.locator('imt-notes-panel').elementHandle();
  await page.evaluate(
    ([row, panel]) => {
      const dt = new DataTransfer();
      const fire = (el: Element, type: string) => el.dispatchEvent(new DragEvent(type, { dataTransfer: dt, bubbles: true, cancelable: true }));
      fire(row as Element, 'dragstart');
      fire(panel as Element, 'dragenter');
      fire(panel as Element, 'dragover');
      fire(panel as Element, 'drop');
      fire(row as Element, 'dragend');
    },
    [handle, target] as const,
  );
}

/** Drops a hand-made payload (any types) on the Notes panel. */
async function dropPayload(page: Page, data: Record<string, string>): Promise<void> {
  const target = await page.locator('imt-notes-panel').elementHandle();
  await page.evaluate(
    ([panel, payload]) => {
      const dt = new DataTransfer();
      for (const [type, value] of Object.entries(payload as Record<string, string>)) dt.setData(type, value);
      const fire = (type: string) => (panel as Element).dispatchEvent(new DragEvent(type, { dataTransfer: dt, bubbles: true, cancelable: true }));
      fire('dragover');
      fire('drop');
    },
    [target, data] as const,
  );
}

test.describe('drag an event onto the Notes panel', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
  });

  test('grid rows are draggable', async ({ page }) => {
    await expect(rows(page).first()).toHaveAttribute('draggable', 'true');
  });

  test('dropping a row on the panel opens the note editor on that event', async ({ page }, testInfo) => {
    const message = (await rows(page).nth(3).locator('.msg').innerText()).trim();
    await openNotes(page);
    await dragRowToNotes(page, 3);
    const editor = page.getByRole('dialog', { name: 'Add a note' });
    await expect(editor).toBeVisible();
    await expect(editor.getByTestId('note-anchor')).toContainText(message.split('\n')[0].slice(0, 40));
    await expectNoSeriousViolations(page, testInfo, 'event-drag-note-editor', '[role=dialog]');
    await editor.getByTestId('note-text').fill('Dragged here');
    await editor.getByTestId('note-save').click();
    await expect(editor).toHaveCount(0);
    await expect(page.locator('imt-notes-panel').getByTestId('note-item')).toContainText('Dragged here');
    await expect(rows(page).nth(3).getByTestId('note-glyph')).toBeVisible();
  });

  test('the panel is outlined while an event is dragged over it', async ({ page }) => {
    await openNotes(page);
    const panel = page.locator('imt-notes-panel');
    await dropPayload(page, {});
    await expect(panel).not.toHaveClass(/drop-target/);
    const target = await panel.elementHandle();
    await page.evaluate((p) => {
      const dt = new DataTransfer();
      dt.setData('application/x-imt-event', JSON.stringify({ id: 1, ts: null, source: 's', excerpt: 'x' }));
      (p as Element).dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }));
    }, target);
    await expect(panel).toHaveClass(/drop-target/);
  });

  test('a malformed payload is ignored: no editor opens', async ({ page }) => {
    await openNotes(page);
    const editor = page.getByRole('dialog', { name: 'Add a note' });
    const type = 'application/x-imt-event';
    for (const data of [
      { [type]: '{not json' },
      { [type]: '[]' },
      { [type]: JSON.stringify({ id: -1 }) },
      { [type]: JSON.stringify({ id: 1.5 }) },
      { [type]: JSON.stringify({ id: '7' }) },
      { [type]: 'null' },
      { 'text/plain': 'just some text' },
    ]) {
      await dropPayload(page, data);
      await expect(editor).toHaveCount(0);
    }
    await expect(page.getByTestId('note-item')).toHaveCount(0);
  });
});
