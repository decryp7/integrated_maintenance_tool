import { expect, Page, test } from '@playwright/test';
import { CAPTURE_TIMEOUT, openApp } from './app';

// Report a problem (mock backend): an error raises the toast, "Report a problem…" opens the dialog that lists
// what the zip will contain and the steps to reproduce, and Save writes it (the mock records the save).

async function runFromPalette(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill(title);
  await page.keyboard.press('Enter');
}

test('an error leads to a problem report the user reviews and saves', async ({ page }) => {
  await openApp(page);
  // Some activity to reproduce.
  await runFromPalette(page, 'Show error snapshots');
  await expect(page.locator('debrief-error-snapshots-view')).toBeVisible();

  await page.evaluate(() => void setTimeout(() => { throw new Error('e2e report boom'); }, 0));
  const toast = page.locator('[data-testid=snapshot-toast]');
  await expect(toast).toContainText('Something went wrong', { timeout: CAPTURE_TIMEOUT });
  await toast.getByRole('button', { name: 'Report a problem…' }).click();

  const dialog = page.getByRole('dialog', { name: 'Report a problem' });
  await expect(dialog).toBeVisible();
  const contents = dialog.locator('[data-testid=report-contents]');
  for (const name of ['description.txt', 'steps-to-reproduce.txt', 'system-info.json', 'logs/activity.log', 'logs/debrief.log']) {
    await expect(contents).toContainText(name);
  }
  // The snapshot of the error is part of the report.
  await expect(contents).toContainText(/snapshots\/.+\.png/);
  await expect(dialog.getByRole('heading', { name: 'Steps to reproduce' })).toBeVisible();
  await expect(dialog.locator('[data-testid=report-steps]')).toContainText('Show error snapshots');
  // Privacy: the preview names files, it does not show log content.
  await expect(dialog).not.toContainText('e2e report boom');

  await dialog.getByRole('textbox', { name: /What happened/ }).fill('The export failed');
  await expect(contents).toContainText('Your description');
  await dialog.getByRole('button', { name: 'Save report…' }).click();
  // Saved by a job: the dialog may close; the message says when the zip is ready, with "Show in folder".
  await expect(page.getByTestId('notice-toast').filter({ hasText: 'Saving the problem report — running in Jobs' })).toBeVisible();
  await expect(dialog.locator('[data-testid=report-saved]')).toContainText('debrief-problem-report.zip');
  await dialog.getByRole('button', { name: 'Close' }).first().click();
  await expect(dialog).toBeHidden();
  const done = page.getByTestId('notice-toast').filter({ hasText: 'Finished: Save problem report debrief-problem-report.zip' });
  await expect(done).toBeVisible({ timeout: 15_000 });
  await expect(done).toContainText('files zipped.');
  await expect(done.getByTestId('notice-toast-primary')).toHaveText('Show in folder');
});

test('Help commands are in the palette: report a problem and show the activity log', async ({ page }) => {
  await openApp(page);
  await runFromPalette(page, 'Report a problem');
  const dialog = page.getByRole('dialog', { name: 'Report a problem' });
  await expect(dialog.locator('[data-testid=report-contents]')).toContainText('system-info.json');
  // The report also carries the manifest, the child-process summary and the child logs.
  for (const name of ['manifest.json', 'processes.json', 'session-recovery.json', 'logs/children/ai-worker.log', 'logs/children/llama-server.log']) {
    await expect(dialog.locator('[data-testid=report-contents]')).toContainText(name);
  }
  await expect(dialog.locator('[data-testid=report-contents]')).toContainText('AI engine: 2 runs, 1 restart, last exit code 3');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toBeHidden();
  // In a browser there is no second window: the command runs without an error.
  await runFromPalette(page, 'Show activity log');
  await expect(page.getByText('Could not open the activity log')).toHaveCount(0);
});
