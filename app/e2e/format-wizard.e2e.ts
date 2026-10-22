import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

test('format wizard: mark a sample line, preview the parse and save', async ({ page }, testInfo) => {
  await openApp(page);
  await page.getByRole('button', { name: /^Set up format for / }).first().click();
  const dialog = page.getByRole('dialog', { name: 'Set up format' });
  await expect(dialog).toBeVisible();
  const sample = dialog.getByTestId('sample-line');
  await expect(sample).toContainText('[INF]');

  const mark = async (needle: string, button: RegExp) => {
    await page.evaluate((text) => {
      const spans = [...document.querySelectorAll('[data-testid=sample-line] span')];
      const node = spans.find((s) => s.textContent?.includes(text))!.firstChild as Text;
      const start = node.data.indexOf(text);
      const range = document.createRange();
      range.setStart(node, start);
      range.setEnd(node, start + text.length);
      const sel = document.getSelection()!;
      sel.removeAllRanges();
      sel.addRange(range);
    }, needle);
    const target = dialog.getByRole('button', { name: button });
    await target.click();
    await expect(target).toContainText('✓');
  };
  await mark('2026-10-14 10:41:02.114 +00:00', /^Timestamp/);
  await mark('INF', /^Level/);
  await mark('Order 1042 accepted for customer 77', /^Message/);

  await expect(dialog.getByTestId('format-regex')).toContainText('(?P<timestamp>');
  // The engine (here its mock) drafts a parser for the file and reads the sample with it.
  await expect(dialog.getByTestId('format-summary')).toContainText('4 events parsed', { timeout: 15_000 });
  await expect(dialog.getByRole('table', { name: 'Parse preview' })).toBeVisible();
  await expect(dialog.getByRole('table', { name: 'Parse preview' })).toContainText('Payment capture failed for order 1042', { timeout: 15_000 });
  await expectNoSeriousViolations(page, testInfo, 'format-wizard');
  await dialog.getByRole('button', { name: 'Save parser and re-read' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: /Saved the parser .*; the files are read again./ })).toBeVisible();
});

test('format wizard: a Serilog output template', async ({ page }) => {
  await openApp(page);
  await page.getByRole('button', { name: /^Set up format for / }).first().click();
  const dialog = page.getByRole('dialog', { name: 'Set up format' });
  await dialog.getByRole('button', { name: 'Serilog output template' }).click();
  await dialog.getByLabel('Output template').fill('{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}');
  await expect(dialog.getByTestId('format-summary')).toContainText('4 events parsed', { timeout: 15_000 });
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
});

test('format wizard: a file outside the workspace cannot be previewed', async ({ page }) => {
  await openApp(page);
  await page.getByRole('button', { name: /^Set up format for / }).first().click();
  const dialog = page.getByRole('dialog', { name: 'Set up format' });
  await expect(dialog.getByTestId('sample-line')).toBeVisible();
  await expect(dialog.getByRole('alert')).toHaveCount(0);
});
