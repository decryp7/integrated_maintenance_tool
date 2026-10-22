import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu, showView } from './menu-support';
import { openAppWith } from './mock-url';

// The Store pane and its SQL console (ADR 0030) against the mock backend: the reports, a confirmed maintenance job,
// and a SQL run with completion, a refusal, results, Open in Events and saving next to the saved queries.
test('Store pane: reports, maintenance and the read-only SQL console', async ({ page }, testInfo) => {
  await openAppWith(page, '');
  await chooseMenu(page, 'View', 'Store');
  const pane = page.locator('imt-store-pane');
  await expect(pane.getByTestId('store-overview')).toContainText('Size on disk');
  await expect(pane.getByTestId('store-activity')).toHaveText('Ready');
  await expectNoSeriousViolations(page, testInfo, 'store-pane', 'imt-store-pane');

  // Maintenance asks first.
  await pane.getByTestId('store-maintain-clearCachedAnalyses').click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Forget the cached analyses?');
  // By keyboard: Cancel has the focus, the confirming button is next.
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(dialog.getByRole('button', { name: 'Clear' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  await expect(pane.getByTestId('store-notice')).toContainText('Started');

  await pane.getByRole('tab', { name: 'Segments' }).click();
  await expect(pane.getByTestId('store-segments').locator('tbody tr').first()).toBeVisible();
  await pane.getByRole('tab', { name: 'Fields' }).click();
  await expect(pane.getByTestId('store-fields').locator('tbody tr').first()).toBeVisible();
  await pane.getByRole('tab', { name: 'Memory' }).click();
  await expect(pane.getByRole('meter').first()).toBeVisible();

  // The SQL console, from the palette.
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('Open SQL console');
  await page.getByRole('option', { name: /Open SQL console/ }).first().click();
  const editor = pane.getByTestId('sql-editor');
  const openConsole = () =>
    expect(async () => {
      await page.keyboard.press('ControlOrMeta+k');
      await page.getByRole('combobox', { name: 'Type a command' }).fill('Open SQL console');
      await page.getByRole('option', { name: /Open SQL console/ }).first().click();
      await expect(editor).toBeVisible();
      await page.waitForTimeout(400);
      await expect(editor).toBeVisible({ timeout: 1000 });
    }).toPass({ timeout: 15_000 });
  await expect(editor).toBeVisible();
  await editor.fill('');
  await editor.pressSequentially('SELECT id, ts, level, message FROM ev');
  await expect(pane.getByTestId('sql-completions')).toContainText('events');
  await editor.press('Tab');
  await expect(editor).toHaveValue('SELECT id, ts, level, message FROM events');
  await editor.press('ControlOrMeta+Enter');
  const rows = pane.getByTestId('sql-results').locator('tbody tr');
  await expect(rows.first()).toBeVisible();
  await expect(pane.getByTestId('sql-count')).toContainText('rows');
  await rows.nth(2).click();
  await pane.getByTestId('sql-open-events').click();
  await expect(page.locator('[role=row][aria-selected=true]').first()).toBeVisible({ timeout: 10_000 });
  // Back to the console (the Events grid came to the front).
  // The slid-out panel settles a moment after the command (it can be dismissed by the focus change of the click that
  // chose it), so the console counts as open once it has stayed visible.
  await openConsole();

  // A write is refused, with the keyword underlined.
  await editor.fill('DROP VIEW events');
  await pane.getByTestId('sql-run').click();
  await expect(pane.getByTestId('sql-error')).toContainText('DROP is not allowed');
  await expect(pane.getByTestId('sql-error').locator('mark')).toHaveText('DROP');

  // Saved SQL is listed with the saved queries.
  await editor.fill('SELECT count(*) FROM events');
  await pane.getByRole('textbox', { name: 'Name for this SQL' }).fill('All events');
  await pane.getByTestId('sql-save').click();
  await showView(page, 'Saved queries');
  const saved = page.getByTestId('saved-sql');
  await expect(saved).toContainText('All events');
  await expect(saved).toContainText('SQL');
  // The Store panel slides away while another panel has the focus: bring it back before editing.
  await openConsole();
  await editor.fill('');
  await saved.getByRole('button', { name: /Open SQL All events/ }).click();
  await expect(editor).toHaveValue('SELECT count(*) FROM events');
});
