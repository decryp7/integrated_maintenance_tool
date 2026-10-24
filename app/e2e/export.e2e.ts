import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';

test('export dialog counts events, exports through the backend and reports it', async ({ page }, testInfo) => {
  await openApp(page);
  await chooseMenu(page, 'File', 'Export events…');

  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId('exp-format').getByRole('button')).toBeVisible();
  await expect(dialog.getByRole('status')).toContainText('events will be exported');
  const all = parseCount(await dialog.getByRole('status').innerText());
  expect(all).toBeGreaterThan(1000);
  await expectNoSeriousViolations(page, testInfo, 'export-dialog');

  await dialog.getByLabel('Newest first').check();
  await dialog.getByTestId('exp-format').getByRole('button').click();
  await page.getByRole('option', { name: /Compact Log Event Format/ }).click();
  await dialog.getByRole('button', { name: 'Export…' }).click();
  // The dialog closes at once; a job writes the file and its message offers "Show in folder".
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('notification').filter({ hasText: 'Exporting to events.clef — running in Jobs' })).toBeVisible();
  const done = page.getByTestId('notification').filter({ hasText: 'Finished: Export events to events.clef' });
  await expect(done).toBeVisible({ timeout: 15_000 });
  await expect(done).toContainText(`Exported ${all} events.`);
  await expect(done.getByTestId('notification-action-primary')).toHaveText('Show in folder');
});

test('export dialog counts only what matches the current filter and closes with Escape', async ({ page }) => {
  await openApp(page);
  const total = parseCount(await initialTotal(page));
  await page.getByTestId('query-field').fill('timeout');
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
