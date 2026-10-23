import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';
import { openAppWith } from './mock-url';

// A file whose timestamps the engine cannot read (the mock stands in: a path naming "no-timestamps" imports
// without timestamps, with the format the engine would propose; see src/app/core/mock/mock-timestamps.ts).
const PATH = '/logs/legacy-no-timestamps.csv';

test('Problems pane: the status-bar badge opens it, the quick fix is previewed and applied', async ({ page }, testInfo) => {
  await openAppWith(page, `?mockOpen=${encodeURIComponent(PATH)}`);
  await page.locator('debrief-sidebar').getByRole('button', { name: 'Open files' }).click();
  await expect(page.locator('debrief-sidebar').getByTestId('timestamp-warning')).toHaveCount(1, { timeout: 10_000 });

  const badge = page.getByTestId('status-problems');
  await expect(badge).toHaveAttribute('aria-label', /^\d+ problems? in the opened files/);
  await badge.click();
  const pane = page.locator('debrief-problems-panel');
  await expect(pane).toBeVisible();
  // The side-bar tab it slid out of counts the problems.
  await expect(page.locator('.debrief-ah-tab[data-ah-id="problems"]')).toContainText(/Problems \(\d+\)/);

  const tree = pane.getByRole('tree', { name: 'Problems' });
  const untimed = tree.getByRole('treeitem', { name: /Events without a readable timestamp/ });
  await expect(untimed).toHaveCount(1);
  await untimed.click();
  await expect(untimed).toHaveAttribute('aria-selected', 'true');
  const details = pane.getByRole('region', { name: 'Problem details' });
  await expect(details.getByTestId('quick-fix')).toContainText('%a %b %d %H:%M:%S %Y');
  await expect(details.getByRole('button', { name: 'Go to line 2' })).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'problems-pane', 'debrief-problems-panel');

  await details.getByRole('button', { name: 'Preview effect' }).click();
  await expect(details.getByTestId('fix-evaluation')).toHaveText(/^Fixes [\d,]+ of [\d,]+ checked events/);
  await details.getByRole('button', { name: 'Apply and re-read' }).click();
  await expect(pane.getByRole('status').filter({ hasText: /Changed the parser .* and read its files again\./ })).toBeVisible({ timeout: 15_000 });
  await expect(untimed).toHaveCount(0);
});

test('Problems pane: reachable from the View menu and the command palette', async ({ page }) => {
  await openAppWith(page, '');
  await chooseMenu(page, 'View', 'Problems');
  await expect(page.locator('debrief-problems-panel')).toBeVisible();
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill('View: Problems');
  await page.getByRole('option', { name: /View: Problems/ }).first().click();
  await expect(page.locator('debrief-problems-panel')).toBeVisible();
  // The demo's unread file: "Ask the AI to fix" and the format wizard are offered.
  const unread = page.locator('debrief-problems-panel').getByRole('treeitem', { name: /no parser reads it/ }).first();
  await unread.click();
  await expect(page.getByRole('region', { name: 'Problem details' }).getByRole('button', { name: 'Ask the AI to fix…' })).toBeVisible();
});
