import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// A file whose timestamps the engine cannot read (the mock stands in: a path naming "no-timestamps" imports
// without timestamps, with the format the engine would propose; see src/app/core/mock/mock-timestamps.ts).
const PATH = '/logs/legacy-no-timestamps.csv';

test('unreadable timestamps: an Explorer warning and one message whose Fix timestamps opens the wizard on the proposed format', async ({ page }, testInfo) => {
  await openAppWith(page, `?mockOpen=${encodeURIComponent(PATH)}`);
  const sidebar = page.locator('debrief-sidebar');
  await sidebar.getByRole('button', { name: 'Open files' }).click();

  const warning = sidebar.getByTestId('timestamp-warning');
  await expect(warning).toHaveCount(1, { timeout: 10_000 });
  await expect(warning).toHaveAttribute('title', /^Timestamps not recognised in \d+ lines \(e\.g\. “Sat Nov 08 00:04:37 2025,INFO,Sync started”\)/);
  await expectNoSeriousViolations(page, testInfo, 'timestamp-warning', 'debrief-sidebar');

  // The same file opened again: still one message.
  await sidebar.getByRole('button', { name: 'Open files' }).click();
  await page.getByTestId('status-notifications').click();
  const pane = page.locator('debrief-notifications-panel');
  const notice = pane.locator('li.notice').filter({ hasText: 'Timestamps not recognised in legacy-no-timestamps.csv' });
  await expect(notice).toHaveCount(1);
  await expect(notice).toContainText('The format %a %b %d %H:%M:%S %Y reads them.');
  await notice.getByRole('button', { name: 'Fix timestamps' }).click();

  const dialog = page.getByRole('dialog', { name: 'Fix timestamps' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId('timestamp-format')).toHaveValue('%a %b %d %H:%M:%S %Y');
  await expect(dialog).toContainText('Debrief checked this format');
  await expect(dialog.getByTestId('format-summary')).toContainText('events parsed', { timeout: 15_000 });
  await expectNoSeriousViolations(page, testInfo, 'fix-timestamps');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);

  // The Explorer icon opens the same wizard.
  await warning.click();
  await expect(page.getByRole('dialog', { name: 'Fix timestamps' }).getByTestId('timestamp-format')).toHaveValue('%a %b %d %H:%M:%S %Y');
});
