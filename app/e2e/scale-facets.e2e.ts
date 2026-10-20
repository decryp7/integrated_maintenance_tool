import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';
import { expandFields } from './app';

test.describe('facet values with 10,000 distinct values', () => {
  test.beforeEach(async ({ page }) => {
    await openAppWith(page, '?mockFacet=10000');
    await expandFields(page);
    await page.getByRole('textbox', { name: 'Filter fields' }).first().fill('Machine');
    await page.locator('imt-sidebar .field').first().click();
    await expect(page.locator('imt-facet-popover')).toBeVisible();
  });

  test('renders a window of values and says how many exist', async ({ page }, testInfo) => {
    const pop = page.locator('imt-facet-popover');
    await expect(pop.getByTestId('facet-summary')).toHaveText('10,000+ values in current results · top 1,000 shown');
    const rendered = await pop.locator('.value').count();
    expect(rendered).toBeGreaterThan(5);
    expect(rendered).toBeLessThan(30);
    await expectNoSeriousViolations(page, testInfo, 'scale-facets', 'imt-facet-popover');
  });

  test('scrolling reaches the last loaded value', async ({ page }) => {
    const pop = page.locator('imt-facet-popover');
    await pop.getByRole('list', { name: 'Values' }).evaluate((el) => {
      el.scrollTop = el.scrollHeight;
    });
    await expect(pop.locator('.value').last()).toContainText('value-00999');
  });

  test('search as you type narrows the list, and a value can be used as a filter', async ({ page }) => {
    const pop = page.locator('imt-facet-popover');
    await pop.getByRole('searchbox', { name: 'Search values' }).fill('00042');
    await expect(pop.locator('.value')).toHaveCount(1);
    await expect(pop.getByTestId('facet-summary')).toContainText('1 match');
    await pop.getByRole('button', { name: /Filter .* equals .*00042/ }).click();
    await expect(page.locator('imt-facet-popover')).toHaveCount(0);
    await expect(page.locator('imt-filter-bar')).toContainText('00042');
  });

  test('says when nothing matches', async ({ page }) => {
    const pop = page.locator('imt-facet-popover');
    await pop.getByRole('searchbox', { name: 'Search values' }).fill('nope-nope');
    await expect(pop).toContainText('No value matches the search.');
  });
});

test('the columns list stays small even with many fields', async ({ page }) => {
  await openAppWith(page, '');
  await page.getByRole('button', { name: 'Columns' }).click();
  const list = page.getByRole('list', { name: 'Columns' });
  await expect(list).toBeVisible();
  expect(await list.locator('[role=listitem]').count()).toBeLessThan(30);
  await page.locator('imt-events-grid .cols').getByRole('textbox', { name: 'Filter fields' }).fill('machine');
  await expect(list.locator('[role=listitem]')).toHaveCount(1);
});
