import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand } from './tools-support';

test('Configuration diff compares two files side by side and unified, filters, folds and exports', async ({ page }, testInfo) => {
  await openAppWith(page);
  await runCommand(page, 'Compare configuration files');
  const panel = page.locator('imt-config-diff-view');
  await expect(panel.getByText('Choose two configuration files to compare.')).toBeVisible();
  const choose = panel.getByRole('button', { name: 'Choose…' });
  await choose.first().click();
  await choose.last().click();
  const tree = panel.getByRole('treegrid', { name: 'Configuration diff' });
  await expect(tree).toHaveAttribute('aria-rowcount', '219');
  await expect(tree.getByRole('row').filter({ hasText: 'Server=db02;Database=Orders' })).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'config-diff');

  // Changes only: unchanged rows disappear, their sections stay as context.
  await panel.getByRole('checkbox', { name: /Unchanged/ }).uncheck();
  await expect(tree).toHaveAttribute('aria-rowcount', /^\d\d$/);
  await expect(tree.locator('[role=row][data-kind=unchanged]:not(.ctx)')).toHaveCount(0);

  // Search, then fold a section.
  await panel.getByRole('searchbox', { name: 'Search paths and values' }).fill('web02');
  await expect(tree).toHaveAttribute('aria-rowcount', '3');
  await tree.getByRole('button', { name: 'Collapse Servers', exact: true }).click();
  await expect(tree).toHaveAttribute('aria-rowcount', '1');
  await tree.getByRole('button', { name: 'Expand Servers', exact: true }).click();
  await expect(tree).toHaveAttribute('aria-rowcount', '3');

  // Unified layout and keyboard selection, then copy the path.
  await panel.getByRole('button', { name: 'Unified' }).click();
  await tree.focus();
  await page.keyboard.press('End');
  await expect(panel.getByRole('group', { name: 'Selected row' })).toContainText('Servers[1]/host');
  await expectNoSeriousViolations(page, testInfo, 'config-diff-unified');

  await panel.getByRole('button', { name: 'Export HTML' }).click();
  await expect(panel.getByRole('status')).toContainText('to /demo/exports/config-diff.html');
});

test('Configuration diff scrolls a large diff in pages and reports unreadable files with Retry', async ({ page }) => {
  await openAppWith(page);
  await runCommand(page, 'Compare configuration files');
  const panel = page.locator('imt-config-diff-view');
  await panel.getByRole('textbox').nth(0).fill('/demo/config/broken.json');
  await panel.getByRole('textbox').nth(0).press('Tab');
  await panel.getByRole('textbox').nth(1).fill('/demo/config/right.json');
  await panel.getByRole('textbox').nth(1).press('Tab');
  await panel.getByRole('button', { name: 'Compare', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('left file: JSON');
  await panel.getByRole('textbox').nth(0).fill('/demo/config/left.json');
  await panel.getByRole('textbox').nth(0).press('Tab');
  await panel.getByRole('button', { name: 'Retry' }).click();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  const tree = panel.getByRole('treegrid', { name: 'Configuration diff' });
  // Only a window of rows is in the DOM; scrolling to the end loads the last page.
  expect(await tree.getByRole('row').count()).toBeLessThan(80);
  await tree.evaluate((e) => (e.scrollTop = e.scrollHeight));
  await expect(tree.getByRole('row').filter({ hasText: 'Timeout' })).toBeVisible();
});

test('Configuration diff compares two snapshot folders and opens a changed file', async ({ page }, testInfo) => {
  await openAppWith(page);
  await runCommand(page, 'Compare configuration files');
  const panel = page.locator('imt-config-diff-view');
  await panel.getByRole('button', { name: 'Two snapshot folders' }).click();
  await panel.getByRole('button', { name: 'Choose…' }).first().click();
  await panel.getByRole('button', { name: 'Choose…' }).last().click();
  const files = panel.getByRole('table', { name: 'Configuration files in both snapshots' });
  await expect(files.getByRole('row')).toHaveCount(7);
  await expect(files).toContainText('EOF while parsing');
  await expectNoSeriousViolations(page, testInfo, 'config-diff-folders');
  await files.getByRole('button', { name: 'Show diff of app/appsettings.json' }).click();
  await expect(panel.getByRole('treegrid', { name: 'Configuration diff' })).toHaveAttribute('aria-rowcount', '219');
  await expect(panel.locator('.sides')).toContainText('appsettings.json');
});
