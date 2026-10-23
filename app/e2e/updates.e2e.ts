import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { chooseMenu } from './menu-support';
import { openAppWith } from './mock-url';

/** Help ▸ Check for updates… against the simulated updater (`?mockUpdate=`, see mock-updates-backend.ts). */
async function checkForUpdates(page: Page) {
  await chooseMenu(page, 'Help', 'Check for updates…');
  const dialog = page.getByRole('dialog', { name: 'Check for updates' });
  await expect(dialog).toBeVisible();
  return dialog;
}

test.describe('Updates', () => {
  test('check, see notes and size, download with progress, restart to update', async ({ page }) => {
    await openApp(page);
    const dialog = await checkForUpdates(page);
    await expect(dialog.getByTestId('updates-available')).toHaveText('Debrief 0.2.0 is available.');
    // Notes are plain text: the markup in them is shown, never rendered.
    const notes = dialog.getByRole('region', { name: 'Release notes' });
    await expect(notes).toContainText('<b>Faster</b> grid scrolling');
    await expect(notes.locator('b')).toHaveCount(0);
    await expect(dialog).toContainText('46 MB');
    await expect(dialog.getByTestId('updates-auto')).toContainText('Automatic checks are off');

    await dialog.getByRole('button', { name: 'Download' }).click();
    await expect(dialog.getByRole('progressbar', { name: 'Download progress' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeVisible();
    await expect(dialog.getByTestId('updates-ready')).toContainText('signature verified');

    await dialog.getByLabel('Install when Debrief quits instead').check();
    await expect(dialog.getByLabel('Install when Debrief quits instead')).toBeChecked();
    await dialog.getByRole('button', { name: 'Restart to update' }).click();
    await expect(dialog).toContainText('Restarting to install Debrief 0.2.0');
  });

  test('a download can be cancelled and the update stays available', async ({ page }) => {
    await openApp(page);
    const dialog = await checkForUpdates(page);
    await dialog.getByRole('button', { name: 'Download' }).click();
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog.getByTestId('updates-available')).toBeVisible();
  });

  test('offline: a typed error with the next step and a new check', async ({ page }) => {
    await openAppWith(page, '?mockUpdate=offline');
    const dialog = await checkForUpdates(page);
    const error = dialog.getByRole('alert');
    await expect(error).toHaveAttribute('data-kind', 'offline');
    await expect(error).toContainText('Debrief could not reach the network.');
    await expect(error).toContainText('Check your internet connection');
    await expect(dialog.getByRole('button', { name: 'Check again' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Open releases page' })).toBeVisible();
  });

  test('server unreachable', async ({ page }) => {
    await openAppWith(page, '?mockUpdate=unreachable');
    const dialog = await checkForUpdates(page);
    await expect(dialog.getByRole('alert')).toHaveAttribute('data-kind', 'unreachable');
    await expect(dialog.getByRole('alert')).toContainText('operation timed out');
  });

  test('an invalid signature is refused and not retried blindly', async ({ page }) => {
    await openAppWith(page, '?mockUpdate=signatureInvalid');
    const dialog = await checkForUpdates(page);
    await dialog.getByRole('button', { name: 'Download' }).click();
    const error = dialog.getByRole('alert');
    await expect(error).toHaveAttribute('data-kind', 'signatureInvalid');
    await expect(error).toContainText('refused to install it');
    await expect(dialog.getByRole('button', { name: 'Try the download again' })).toHaveCount(0);
    await expect(dialog.getByRole('button', { name: 'Check again' })).toBeVisible();
  });

  test('disk full offers the download again', async ({ page }) => {
    await openAppWith(page, '?mockUpdate=diskFull');
    const dialog = await checkForUpdates(page);
    await dialog.getByRole('button', { name: 'Download' }).click();
    await expect(dialog.getByRole('alert')).toContainText('Free some disk space');
    await expect(dialog.getByRole('button', { name: 'Try the download again' })).toBeVisible();
  });

  test('a build without an update key says updates are not available', async ({ page }) => {
    await openAppWith(page, '?mockUpdate=unavailable');
    const dialog = await checkForUpdates(page);
    await expect(dialog.getByTestId('updates-unavailable')).toHaveText('Updates are not available in this build.');
    await expect(dialog.getByRole('button', { name: 'Download' })).toHaveCount(0);
  });

  test('up to date', async ({ page }) => {
    await openAppWith(page, '?mockUpdate=upToDate');
    const dialog = await checkForUpdates(page);
    await expect(dialog.getByTestId('updates-up-to-date')).toHaveText('Debrief is up to date.');
  });
});
