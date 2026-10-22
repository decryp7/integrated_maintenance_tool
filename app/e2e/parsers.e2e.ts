import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// Settings ▸ Parsers (ADR 0026) against the browser build's mock parser library
// (src/app/core/mock/mock-parsers.ts): "Import .toml…" picks /demo/parsers/inventory-csv.toml, an invented parser
// for the demo's inventory-sync.csv; exports go to /demo/exports.

const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const row = (page: Page, id: string) => settingsDialog(page).locator(`li[data-parser="${id}"]`);

async function openParsers(page: Page) {
  await page.keyboard.press('ControlOrMeta+Comma');
  await expect(settingsDialog(page)).toBeVisible();
  await settingsDialog(page).getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Parsers' }).click();
  await expect(settingsDialog(page).getByRole('heading', { name: 'Parsers' })).toBeVisible();
}

test('imports a parser file, re-reads the file it now reads, disables and exports it', async ({ page }, testInfo) => {
  await openApp(page);
  await openParsers(page);
  await expect(row(page, 'serilog-clef')).toContainText('Built-in');
  await expect(settingsDialog(page).getByTestId('parser-read-serilog-clef')).toHaveText(/^Read 2 files: orders-20261014\.clef/);
  await expect(settingsDialog(page).getByTestId('parser-read-csv')).toHaveText('Read 1 file: inventory-sync.csv');

  await settingsDialog(page).getByRole('button', { name: 'Import .toml…' }).click();
  await expect(settingsDialog(page).getByTestId('parsers-notice')).toHaveText(/Imported Inventory sync CSV 1\.0\.0\./);
  await expect(row(page, 'inventory-csv')).toContainText('Custom');
  await expect(row(page, 'inventory-csv')).toContainText('csv');

  // The new parser reads inventory-sync.csv better than the built-in that read it: re-read it.
  const stale = settingsDialog(page).getByTestId('parsers-stale');
  await expect(stale).toContainText('1 source was read with a parser that changed.');
  await stale.getByRole('button', { name: 'Re-read' }).click();
  await expect(settingsDialog(page).getByTestId('parser-read-inventory-csv')).toHaveText('Read 1 file: inventory-sync.csv');
  await expect(stale).toHaveCount(0);
  await expect(settingsDialog(page).getByTestId('parser-read-csv')).toHaveText('Read no files in this window');
  await expectNoSeriousViolations(page, testInfo, 'settings-parsers', '[role=dialog]');

  // Disabled, the file it read is stale again.
  const enabled = row(page, 'inventory-csv').getByRole('switch', { name: 'Use Inventory sync CSV' });
  await expect(enabled).toBeChecked();
  await enabled.click();
  await expect(enabled).not.toBeChecked();
  await expect(stale).toContainText('1 source was read with a parser that changed.');

  // Export, then export again over the same file (asks first).
  await row(page, 'inventory-csv').getByRole('button', { name: 'Export… Inventory sync CSV' }).click();
  await expect(settingsDialog(page).getByTestId('parsers-notice')).toContainText('Exported inventory-csv to /demo/exports/inventory-csv.toml.');
  await row(page, 'inventory-csv').getByRole('button', { name: 'Export… Inventory sync CSV' }).click();
  const replace = page.getByRole('dialog', { name: 'Replace file?' });
  await replace.getByRole('button', { name: 'Replace' }).click();
  await expect(replace).toHaveCount(0);
  await expect(settingsDialog(page).getByTestId('parsers-notice')).toContainText('Exported inventory-csv to /demo/exports/inventory-csv.toml.');
});

test('edits a built-in parser in the TOML editor with validation and a live preview', async ({ page }) => {
  await openApp(page);
  await openParsers(page);
  await row(page, 'serilog-text').getByRole('button', { name: 'Edit Serilog text' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit parser serilog-text' });
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
  await expect(settingsDialog(page).getByTestId('parsers-stale')).toContainText(/read with (a parser|parsers) that changed\./);
});
