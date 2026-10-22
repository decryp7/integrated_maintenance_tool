import { expect, test } from '@playwright/test';
import { expandFields, openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openQuery, runCommand, searchField } from './analysis-support';

test.describe('analysis views', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('Moment view summarises the current range', async ({ page }, testInfo) => {
    await runCommand(page, 'Analyse this moment');
    const view = page.locator('imt-moment-view');
    await expect(view).toBeVisible();
    await expect(view.getByText(/\d[\d,]* events/).first()).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Instances' })).toBeVisible();
    await expect(view.locator('.card').first()).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Metrics' })).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Users' })).toBeVisible();
    await expect(view.getByRole('heading', { name: 'User activity' })).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Anomalies in this window' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'moment', 'imt-moment-view');
  });

  test('Moment: first error click selects the record in the inspector', async ({ page }) => {
    await runCommand(page, 'Analyse this moment');
    const view = page.locator('imt-moment-view');
    const go = view.getByRole('button', { name: 'Go to first error' }).first();
    await expect(go).toBeVisible();
    await go.click();
    await expect(page.locator('imt-inspector')).toContainText(/Error|Fatal/);
  });

  test('Anomalies: clicking a finding zooms the range and selects a sample', async ({ page }, testInfo) => {
    await runCommand(page, 'Show anomalies');
    const view = page.locator('imt-anomalies-view');
    const rows = view.getByRole('list', { name: 'Anomalies' }).getByRole('button');
    await expect(rows.first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'anomalies', 'imt-anomalies-view');
    await rows.first().click();
    // The range chip of the filter bar shows the zoomed range with a way to reset it.
    await expect(page.locator('imt-filter-bar')).toContainText(/\d{2}:\d{2}/);
    await expect(page.locator('imt-inspector')).toBeVisible();
    await expect(page.locator('imt-inspector')).toContainText(/Information|Warning|Error|Fatal|Debug/);
  });

  test('Patterns and Exceptions list engine results', async ({ page }, testInfo) => {
    await runCommand(page, 'Show patterns');
    const patterns = page.locator('imt-patterns-view');
    await expect(patterns.getByRole('list', { name: 'Patterns' }).getByRole('button').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'patterns', 'imt-patterns-view');

    await runCommand(page, 'Show exceptions');
    const exceptions = page.locator('imt-exceptions-view');
    const head = exceptions.locator('.head').first();
    await expect(head).toBeVisible();
    await head.click();
    await expect(head).toHaveAttribute('aria-expanded', 'true');
    await expect(exceptions.getByRole('heading', { name: 'Sample events' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'exceptions', 'imt-exceptions-view');
  });

  test('Error outliers: values concentrated among errors filter the events', async ({ page }, testInfo) => {
    await runCommand(page, 'Show error outliers');
    const view = page.locator('imt-outliers-view');
    const rows = view.getByRole('list', { name: 'Error outliers' }).getByRole('button', { name: /×/ });
    await expect(rows.first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'outliers', 'imt-outliers-view');
    await rows.first().click();
    await expect(searchField(page)).toHaveValue(/@level >= error/);
  });

  test('Compare: current range against the previous one', async ({ page }, testInfo) => {
    await runCommand(page, 'Compare: before / after selected time');
    const view = page.locator('imt-compare-view');
    await expect(view).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Summary' })).toBeVisible();
    await expect(view.getByText(/Window B has/)).toBeVisible();
    await expect(view.locator('.card')).toHaveCount(2);
    await expect(view.getByRole('heading', { name: 'Numeric changes' })).toBeVisible();
    await view.getByRole('button', { name: 'Current range vs previous' }).click();
    await expect(view.locator('.windows')).toContainText('A');
    await expectNoSeriousViolations(page, testInfo, 'compare', 'imt-compare-view');
  });

  test('Field statistics: histogram, percentiles and top values', async ({ page }, testInfo) => {
    await runCommand(page, 'Field statistics…');
    const view = page.locator('imt-field-stats-view');
    await view.getByRole('combobox', { name: 'Field' }).selectOption('ElapsedMs');
    await expect(view.getByRole('img', { name: /Histogram of ElapsedMs/ })).toBeVisible();
    await expect(view.locator('canvas').first()).toBeVisible();
    await expect(view.locator('.pcts')).toContainText('p95');
    await expect(view.getByRole('heading', { name: 'Top values' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'field-stats', 'imt-field-stats-view');
  });

  test('Fields: the per-field Statistics… action opens field statistics for that field', async ({ page }) => {
    await expandFields(page);
    const field = page.locator('imt-fields-panel .field-wrap').filter({ hasText: 'ElapsedMs' }).first();
    await field.hover();
    await field.getByRole('button', { name: 'Statistics for ElapsedMs' }).click();
    const view = page.locator('imt-field-stats-view');
    await expect(view).toBeVisible();
    await expect(view.getByRole('combobox', { name: 'Field' })).toHaveValue('ElapsedMs');
    await expect(view.getByRole('img', { name: /Histogram of ElapsedMs/ })).toBeVisible();
  });

  test('Exceptions: a sample event click-through scrolls the grid to the row and selects it', async ({ page }) => {
    await runCommand(page, 'Show exceptions');
    const exceptions = page.locator('imt-exceptions-view');
    await exceptions.locator('.head').first().click();
    const sample = exceptions.locator('.samples button').first();
    await sample.click();
    await expect(page.locator('imt-events-grid [role=row][aria-selected=true]')).toBeVisible();
    await expect(page.locator('imt-inspector')).toContainText(/Error|Fatal|Exception/);
  });

  test('Patterns: a click ANDs the template with the applied query', async ({ page }) => {
    const input = await openQuery(page);
    await input.click();
    await input.fill('level >= warning');
    await input.press('ControlOrMeta+Enter');
    await expect(page.locator('[data-testid=query-applied]')).toContainText('level >= warning');
    await runCommand(page, 'Show patterns');
    const patterns = page.locator('imt-patterns-view');
    await patterns.getByRole('list', { name: 'Patterns' }).locator('.row').first().click();
    await expect(page.locator('[data-testid=query-applied]')).toContainText(/level >= warning and (@template|@message)/);
  });

  test('Traces & requests: waterfall and request filter', async ({ page }, testInfo) => {
    await runCommand(page, 'Show traces and requests');
    const view = page.locator('imt-traces-view');
    const traces = view.getByRole('list', { name: 'Traces' }).getByRole('button');
    await expect(traces.first()).toBeVisible();
    await traces.first().click();
    await expect(view.locator('.wrow').first()).toBeVisible();
    await expect(view.getByText(/^Total /)).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'traces', 'imt-traces-view');

    await view.getByRole('button', { name: 'Requests' }).click();
    const requests = view.getByRole('region', { name: 'Requests' }).getByRole('button');
    await expect(requests.first()).toBeVisible();
    await requests.first().click();
    await expect(page.locator('imt-filter-bar')).toContainText(/CorrelationId|RequestId|TraceId/);
    await expectNoSeriousViolations(page, testInfo, 'requests', 'imt-traces-view');
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`views have no serious accessibility violations (${scheme})`, async ({ page }, testInfo) => {
      await page.emulateMedia({ colorScheme: scheme });
      await runCommand(page, 'Analyse this moment');
      await expect(page.locator('imt-moment-view .card').first()).toBeVisible();
      await expectNoSeriousViolations(page, testInfo, `moment-${scheme}`, 'imt-moment-view');
      await runCommand(page, 'Show anomalies');
      await expect(page.locator('imt-anomalies-view li').first()).toBeVisible();
      await expectNoSeriousViolations(page, testInfo, `anomalies-${scheme}`, 'imt-anomalies-view');
    });
  }
});
