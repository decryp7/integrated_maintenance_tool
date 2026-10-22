import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';
import { expectNoSeriousViolations } from './axe';

test('an empty grid says why and offers the next step', async ({ page }, testInfo) => {
  await openApp(page);
  const total = await initialTotal(page);
  await page.getByTestId('query-field').fill('zzz-no-such-text-zzz');
  const grid = page.getByRole('grid', { name: 'Events' });
  await expect(grid).toContainText('No events match the current filters.');
  await expect(grid).toContainText('Filters may be hiding events.');
  await expectNoSeriousViolations(page, testInfo, 'empty-grid', 'imt-events-grid');
  await grid.getByRole('button', { name: 'Clear all filters' }).click();
  await expect(gridTotal(page)).toHaveText(total);
  await expect(page.getByTestId('query-field')).toHaveValue('');
});

test('an empty analysis view suggests what to do', async ({ page }) => {
  await openApp(page);
  await page.getByTestId('query-field').fill('zzz-no-such-text-zzz');
  await page.keyboard.press('ControlOrMeta+k');
  await page.getByRole('combobox', { name: 'Type a command' }).fill('Show exceptions');
  await page.keyboard.press('Enter');
  const view = page.locator('imt-exceptions-view');
  await expect(view).toContainText('No exceptions in the current query.');
  await expect(view).toContainText('grouped here by type and top frame');
  await view.getByRole('button', { name: 'Clear all filters' }).click();
  await expect(view).not.toContainText('No exceptions in the current query.');
});
