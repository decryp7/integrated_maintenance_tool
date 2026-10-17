import { expect, test } from '@playwright/test';
import { openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';

test('export dialog counts events, exports through the backend and reports it', async ({ page }, testInfo) => {
  await openApp(page);
  await page.getByRole('button', { name: 'File', exact: true }).click();
  await page.getByRole('menuitem', { name: /^Export events/ }).click();

  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('combobox', { name: 'Format' })).toBeFocused();
  await expect(dialog.getByRole('status')).toContainText('events will be exported');
  const all = parseCount(await dialog.getByRole('status').innerText());
  expect(all).toBeGreaterThan(1000);
  await expectNoSeriousViolations(page, testInfo, 'export-dialog');

  await dialog.getByLabel('Newest first').check();
  await dialog.getByLabel('Format').selectOption('clef');
  await dialog.getByRole('button', { name: 'Export…' }).click();
  await expect(dialog).toHaveCount(0);
  const notice = page.getByRole('status').filter({ hasText: 'Exported' });
  await expect(notice).toContainText(`Exported ${all.toLocaleString('en-US')} events to events.clef`);
});

test('export dialog counts only what matches the current filter and closes with Escape', async ({ page }) => {
  await openApp(page);
  const total = parseCount(await page.locator('imt-events-grid .bar .total').innerText());
  await page.getByPlaceholder(/Search messages/).fill('timeout');
  await expect(page.locator('imt-events-grid .bar .total')).not.toHaveText(String(total).replace(/\B(?=(\d{3})+(?!\d))/g, ','));
  const filtered = parseCount(await page.locator('imt-events-grid .bar .total').innerText());
  await page.keyboard.press('ControlOrMeta+Shift+e');
  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await expect(dialog.getByRole('status')).toContainText(`${filtered.toLocaleString('en-US')} events will be exported`);
  await dialog.getByLabel('All events').check();
  await expect(dialog.getByRole('status')).toContainText(`${total.toLocaleString('en-US')} events will be exported`);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
});
