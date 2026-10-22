import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// The Parsers view (ADR 0026, I3) against the browser build's mock parser library
// (src/app/core/mock/mock-parsers.ts): "Import .toml…" picks /demo/parsers/inventory-csv.toml, an invented parser
// for the demo's inventory-sync.csv; exports go to /demo/exports.

const view = (page: Page) => page.locator('debrief-parsers-view');
const row = (page: Page, id: string) => view(page).locator(`li[data-parser="${id}"]`);

async function openParsers(page: Page) {
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('Manage parsers');
  await page.keyboard.press('Enter');
  await expect(view(page).getByRole('heading', { name: 'Parsers' })).toBeVisible();
}

test('imports a parser file, re-reads the file it now reads, disables and exports it', async ({ page }, testInfo) => {
  await openApp(page);
  await openParsers(page);
  await expect(row(page, 'serilog-clef')).toContainText('Built-in');
  await expect(view(page).getByTestId('parser-read-serilog-clef')).toHaveText(/^Read 2 files: orders-20261014\.clef/);
  await expect(view(page).getByTestId('parser-read-csv')).toHaveText('Read 1 file: inventory-sync.csv');

  await view(page).getByRole('button', { name: 'Import .toml…' }).click();
  await expect(view(page).getByTestId('parsers-notice')).toHaveText(/Imported Inventory sync CSV 1\.0\.0\./);
  await expect(row(page, 'inventory-csv')).toContainText('Custom');
  await expect(row(page, 'inventory-csv')).toContainText('csv');

  // The new parser reads inventory-sync.csv better than the built-in that read it: re-read it.
  const stale = view(page).getByTestId('parsers-stale');
  await expect(stale).toContainText('1 source was read with a parser that changed.');
  await stale.getByRole('button', { name: 'Re-read' }).click();
  await expect(view(page).getByTestId('parser-read-inventory-csv')).toHaveText('Read 1 file: inventory-sync.csv');
  await expect(stale).toHaveCount(0);
  await expect(view(page).getByTestId('parser-read-csv')).toHaveText('Read no files in this window');
  await expectNoSeriousViolations(page, testInfo, 'parsers-view', 'debrief-parsers-view');

  // Disabled, the file it read is stale again.
  const enabled = row(page, 'inventory-csv').getByRole('switch', { name: 'Use Inventory sync CSV' });
  await expect(enabled).toBeChecked();
  await enabled.click();
  await expect(enabled).not.toBeChecked();
  await expect(stale).toContainText('1 source was read with a parser that changed.');

  // Export, then export again over the same file (asks first).
  await row(page, 'inventory-csv').getByRole('button', { name: 'Export… Inventory sync CSV' }).click();
  await expect(view(page).getByTestId('parsers-notice')).toContainText('Exported inventory-csv to /demo/exports/inventory-csv.toml.');
  await row(page, 'inventory-csv').getByRole('button', { name: 'Export… Inventory sync CSV' }).click();
  const replace = page.getByRole('dialog', { name: 'Replace file?' });
  await replace.getByRole('button', { name: 'Replace' }).click();
  await expect(replace).toHaveCount(0);
  await expect(view(page).getByTestId('parsers-notice')).toContainText('Exported inventory-csv to /demo/exports/inventory-csv.toml.');
});

test('edits a built-in parser beside the list, with validation and a live preview', async ({ page }) => {
  await openApp(page);
  await openParsers(page);
  await row(page, 'serilog-text').getByRole('button', { name: 'Edit Serilog text' }).click();
  // The editor is the view's detail pane, not a dialog: the list stays in view.
  const editor = view(page).locator('debrief-parser-editor');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(row(page, 'serilog-text')).toBeVisible();
  await expect(editor.getByTestId('parser-builtin-note')).toContainText('override');
  await expect(editor.getByTestId('parser-validation')).toContainText('Valid: Serilog text 1.0.0');
  await expect(editor.getByTestId('parser-preview-summary')).toContainText('events read from the start of the file');
  const toml = editor.getByRole('textbox', { name: 'Parser TOML' });
  await toml.fill((await toml.inputValue()).replace('version = "1.0.0"', 'version = "one"'));
  await expect(editor.getByText('Line 4, column 1')).toBeVisible();
  await expect(editor.getByRole('button', { name: 'Save as override' })).toBeDisabled();
  await toml.fill((await toml.inputValue()).replace('version = "one"', 'version = "1.0.1"'));
  await expect(editor.getByTestId('parser-validation')).toContainText('Valid: Serilog text 1.0.1');
  await editor.getByRole('button', { name: 'Save as override' }).click();
  await expect(editor).toHaveCount(0);
  await expect(row(page, 'serilog-text')).toContainText('Overrides built-in');
  await expect(view(page).getByTestId('parsers-stale')).toContainText(/read with (a parser|parsers) that changed\./);
});

test('Explorer "Parse with…" opens the Parsers view to choose a parser for that file, not a dialog of its own (I3)', async ({ page }) => {
  await openApp(page);
  const source = page.locator('debrief-sidebar .src').filter({ hasText: 'inventory-sync.csv' });
  await source.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Parse with…' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(view(page).getByTestId('parsers-target')).toContainText('inventory-sync.csv');
  await expect(view(page).getByTestId('parsers-target-auto')).toHaveClass(/primary/);

  await row(page, 'generic').getByTestId('parser-read-with').click();
  await expect(view(page).getByTestId('parsers-target')).toHaveCount(0);
  await expect(view(page).getByTestId('parser-read-generic')).toContainText('inventory-sync.csv');
});
