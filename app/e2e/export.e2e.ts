import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test('export dialog counts events, exports through the backend and reports it', async ({ page }, testInfo) => {
  await openApp(page);
  await chooseMenu(page, 'File', 'Export events…');

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
  const total = parseCount(await initialTotal(page));
  await page.getByPlaceholder(/Search messages/).fill('timeout');
  // The narrowed total itself, read once it has settled (not a transient "different" value).
  let filtered = 0;
  await expect.poll(async () => {
    filtered = parseCount(await gridTotal(page).innerText());
    return filtered > 0 && filtered < total;
  }).toBe(true);
  await page.keyboard.press('ControlOrMeta+Shift+e');
  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await expect(dialog.getByRole('status')).toContainText(`${filtered.toLocaleString('en-US')} events will be exported`);
  await dialog.getByLabel('All events').check();
  await expect(dialog.getByRole('status')).toContainText(`${total.toLocaleString('en-US')} events will be exported`);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
});
