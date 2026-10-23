import { expect, Page, test } from '@playwright/test';
import { openAppWith } from './mock-url';

// Opening a diagnostics zip (mock backend): the engine recognises it, so the Diagnostics report panel shows its
// processes, steps, breadcrumbs and snapshots. `?mockOpen=` makes the browser build's "Open files" pick that path.

const ZIP = '/samples/diagnostics/debrief-diagnostics-sample.zip';
const rangeButton = (page: Page) => page.getByTitle(/Reset to the full range|Drag on the timeline to zoom/);

test('opening a diagnostics zip shows the report with processes, steps, breadcrumbs and snapshots', async ({ page }) => {
  await openAppWith(page, `?mockOpen=${encodeURIComponent(ZIP)}`);
  await expect(page.locator('debrief-diagnostics-report-view')).toHaveCount(0);
  await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();

  const view = page.locator('debrief-diagnostics-report-view');
  await expect(view).toBeVisible();
  await expect(view.getByTestId('diag-header')).toContainText('debrief-diagnostics-sample.zip');

  const processes = view.getByTestId('diag-processes');
  await expect(processes.getByRole('columnheader', { name: 'State' })).toBeVisible();
  await expect(processes.locator('tr[data-state=crashed]')).toContainText('llama-server');
  await expect(processes.locator('tr[data-state=crashed]')).toContainText('exit code 3');
  await expect(processes.locator('tr[data-state=quarantined]')).toContainText('failed 3 times in a row');
  await expect(view.getByTestId('diag-steps').locator('li').first()).toContainText('Opened');

  // The opened zip is a source like any other file.
  await expect(page.locator('debrief-sidebar')).toContainText('debrief-diagnostics-sample.zip');

  // A breadcrumb zooms the timeline to the moment it happened.
  const range = rangeButton(page);
  const before = (await range.textContent()) ?? '';
  await view.getByTestId('diag-breadcrumbs').getByRole('button').nth(1).click();
  await expect(range).not.toHaveText(before);

  // A snapshot opens large, with its image and context.
  await view.getByTestId('diag-snapshot-card').getByRole('button').click();
  const detail = view.getByTestId('diag-snapshot-detail');
  await expect(detail).toContainText('The AI engine stopped unexpectedly');
  await expect(detail.getByTestId('diag-snapshot-image')).toHaveAttribute('src', /^data:image\/png;base64,/);
});

test('without a diagnostics zip the panel explains how to open one', async ({ page }) => {
  await openAppWith(page);
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('Diagnostics report');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('diag-empty')).toContainText('No diagnostics report is open');
});
