import { expect, test } from '@playwright/test';
import { expandFields, openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openQuery, runCommand, searchField } from './analysis-support';

test.describe('analysis views', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('Moment view summarises the current range', async ({ page }, testInfo) => {
    await runCommand(page, 'Analyse this moment');
    const view = page.locator('debrief-moment-view');
    await expect(view).toBeVisible();
    await expect(view.getByText(/\d[\d,]* events/).first()).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Instances' })).toBeVisible();
    await expect(view.locator('.card').first()).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Metrics' })).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Users' })).toBeVisible();
    await expect(view.getByRole('heading', { name: 'User activity' })).toBeVisible();
    await expect(view.getByRole('heading', { name: 'Anomalies in this window' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'moment', 'debrief-moment-view');
  });

  test('Moment: first error click selects the record in the inspector', async ({ page }) => {
    await runCommand(page, 'Analyse this moment');
    const view = page.locator('debrief-moment-view');
    const go = view.getByRole('button', { name: 'Go to first error' }).first();
    await expect(go).toBeVisible();
    await go.click();
    await expect(page.locator('debrief-inspector')).toContainText(/Error|Fatal/);
  });

  test('Anomalies: clicking a finding zooms the range and selects a sample', async ({ page }, testInfo) => {
    await runCommand(page, 'Show anomalies');
    const view = page.locator('debrief-anomalies-view');
    const rows = view.getByRole('list', { name: 'Anomalies' }).getByRole('button');
    await expect(rows.first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'anomalies', 'debrief-anomalies-view');
    await rows.first().click();
    // The range chip of the filter bar shows the zoomed range with a way to reset it.
    await expect(page.locator('debrief-filter-bar')).toContainText(/\d{2}:\d{2}/);
    await expect(page.locator('debrief-inspector')).toBeVisible();
    await expect(page.locator('debrief-inspector')).toContainText(/Information|Warning|Error|Fatal|Debug/);
  });

  test('Patterns and Exceptions list engine results', async ({ page }, testInfo) => {
    await runCommand(page, 'Show patterns');
    const patterns = page.locator('debrief-patterns-view');
    await expect(patterns.getByRole('list', { name: 'Patterns' }).getByRole('button').first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'patterns', 'debrief-patterns-view');

    await runCommand(page, 'Show exceptions');
    const exceptions = page.locator('debrief-exceptions-view');
    const head = exceptions.locator('.head').first();
    await expect(head).toBeVisible();
    await head.click();
    await expect(head).toHaveAttribute('aria-expanded', 'true');
    await expect(exceptions.getByRole('heading', { name: 'Sample events' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'exceptions', 'debrief-exceptions-view');
  });

  test('Patterns: trend sparklines and the values behind placeholders', async ({ page }) => {
    await runCommand(page, 'Show patterns');
    const view = page.locator('debrief-patterns-view');
    await expect(view.getByTestId('pattern-trend').first()).toBeVisible();
    const values = view.getByRole('button', { name: /^Show the values behind the placeholders/ }).first();
    await values.click();
    await expect(view.getByTestId('pattern-values').first()).toContainText(/Value 1 · [\d,]+ events/);
  });

  test('Error outliers: values concentrated among errors filter the events', async ({ page }, testInfo) => {
    await runCommand(page, 'Show error outliers');
    const view = page.locator('debrief-outliers-view');
    const rows = view.getByRole('list', { name: 'Error outliers' }).getByRole('button', { name: /×/ });
    await expect(rows.first()).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'outliers', 'debrief-outliers-view');
    await rows.first().click();
    await expect(searchField(page)).toHaveValue(/@level >= error/);
  });

  test('Compare: current range against the previous one', async ({ page }, testInfo) => {
    await runCommand(page, 'Compare: before / after selected time');
    const view = page.locator('debrief-compare-view');
    await expect(view).toBeVisible();
    await expect(view.getByTestId('compare-summary')).toBeVisible();
    await expect(view.getByText(/Window B has/)).toBeVisible();
    await expect(view.locator('.card')).toHaveCount(2);
    await expect(view.getByRole('heading', { name: 'Numeric changes' })).toBeVisible();
    await view.getByRole('button', { name: 'Current range vs previous' }).click();
    await expect(view.getByTestId('window-a')).toContainText('A:');
    await expectNoSeriousViolations(page, testInfo, 'compare', 'debrief-compare-view');
  });

  test('Field statistics: histogram, percentiles and top values', async ({ page }, testInfo) => {
    await runCommand(page, 'Field statistics…');
    const view = page.locator('debrief-field-stats-view');
    await view.getByRole('combobox', { name: 'Field' }).selectOption('ElapsedMs');
    await expect(view.getByRole('img', { name: /Histogram of ElapsedMs/ })).toBeVisible();
    await expect(view.locator('canvas').first()).toBeVisible();
    await expect(view.locator('.pcts')).toContainText('p95');
    await expect(view.getByRole('heading', { name: 'Top values' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'field-stats', 'debrief-field-stats-view');
  });

  test('Fields: the per-field Statistics… action opens field statistics for that field', async ({ page }) => {
    await expandFields(page);
    const field = page.locator('debrief-fields-panel .field-wrap').filter({ hasText: 'ElapsedMs' }).first();
    await field.hover();
    await field.getByRole('button', { name: 'Statistics for ElapsedMs' }).click();
    const view = page.locator('debrief-field-stats-view');
    await expect(view).toBeVisible();
    await expect(view.getByRole('combobox', { name: 'Field' })).toHaveValue('ElapsedMs');
    await expect(view.getByRole('img', { name: /Histogram of ElapsedMs/ })).toBeVisible();
  });

  test('Exceptions: a sample event click-through scrolls the grid to the row and selects it', async ({ page }) => {
    await runCommand(page, 'Show exceptions');
    const exceptions = page.locator('debrief-exceptions-view');
    await exceptions.locator('.head').first().click();
    const sample = exceptions.locator('.samples button').first();
    await sample.click();
    await expect(page.locator('debrief-events-grid [role=row][aria-selected=true]')).toBeVisible();
    await expect(page.locator('debrief-inspector')).toContainText(/Error|Fatal|Exception/);
  });

  test('Patterns: a click ANDs the template with the applied query', async ({ page }) => {
    const input = await openQuery(page);
    await input.click();
    await input.fill('level >= warning');
    await input.press('ControlOrMeta+Enter');
    await expect(page.locator('[data-testid=query-applied]')).toContainText('level >= warning');
    await runCommand(page, 'Show patterns');
    const patterns = page.locator('debrief-patterns-view');
    await patterns.getByRole('list', { name: 'Patterns' }).locator('.row').first().click();
    await expect(page.locator('[data-testid=query-applied]')).toContainText(/level >= warning and (@template|@message)/);
  });

  test('Traces & requests: waterfall and request filter', async ({ page }, testInfo) => {
    await runCommand(page, 'Show traces and requests');
    const view = page.locator('debrief-traces-view');
    const traces = view.getByRole('list', { name: 'Traces' }).getByRole('button');
    await expect(traces.first()).toBeVisible();
    await traces.first().click();
    await expect(view.locator('.wrow').first()).toBeVisible();
    await expect(view.getByText(/^Total /)).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'traces', 'debrief-traces-view');

    await view.getByRole('button', { name: 'Requests' }).click();
    const requests = view.getByRole('region', { name: 'Requests' }).getByRole('button');
    await expect(requests.first()).toBeVisible();
    await requests.first().click();
    await expect(page.locator('debrief-filter-bar')).toContainText(/CorrelationId|RequestId|TraceId/);
    await expectNoSeriousViolations(page, testInfo, 'requests', 'debrief-traces-view');
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`views have no serious accessibility violations (${scheme})`, async ({ page }, testInfo) => {
      await page.emulateMedia({ colorScheme: scheme });
      await runCommand(page, 'Analyse this moment');
      await expect(page.locator('debrief-moment-view .card').first()).toBeVisible();
      await expectNoSeriousViolations(page, testInfo, `moment-${scheme}`, 'debrief-moment-view');
      await runCommand(page, 'Show anomalies');
      await expect(page.locator('debrief-anomalies-view li').first()).toBeVisible();
      await expectNoSeriousViolations(page, testInfo, `anomalies-${scheme}`, 'debrief-anomalies-view');
    });
  }
});
