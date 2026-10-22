import { expect, Page, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp } from './app';
import { expectNoSeriousViolations } from './axe';

async function runCommand(page: Page, title: string): Promise<void> {
  await page.keyboard.press('ControlOrMeta+k');
  const input = page.getByRole('combobox', { name: 'Type a command' });
  await input.fill(title);
  // Scoped to the palette: an open editor's <select> options are options too.
  await expect(page.getByRole('listbox').getByRole('option').first()).toContainText(title);
  await page.keyboard.press('Enter');
  await expect(input).toHaveCount(0);
}

test.describe('runs between lifecycle markers', () => {
  test.beforeEach(async ({ page }) => openApp(page));

  test('the default rule draws runs per lane, with anomalies, a tooltip and click-to-filter', async ({ page }, testInfo) => {
    const timeline = page.locator('imt-timeline-panel');
    const legend = timeline.getByRole('group', { name: 'Runs' });
    await expect(legend).toContainText('Running');
    const runs = timeline.locator('.lane .run[data-kind="run"]');
    await expect(runs.first()).toBeVisible();

    // Clicking a run filters to its lane and time range.
    // The shared worker file: 2208 restarted without a stop; WEB01's log starts with a stop without start.
    await expect(timeline.locator('.lane .run[data-status="noStop"]')).toHaveCount(1);
    await expect(timeline.locator('.lane .run[data-status="noStart"]')).toHaveCount(1);
    await expect(timeline.locator('.lane imt-run-track .still').first()).toHaveText('still running');
    await expect(runs.first()).toHaveAttribute('tabindex', '-1');
    await expectNoSeriousViolations(page, testInfo, 'timeline-runs');

    const closed = timeline.locator('.lane .run[data-status="closed"]').first();
    await closed.dispatchEvent('pointerenter', { clientX: 400, clientY: 300 });
    const tip = page.getByTestId('run-tip');
    await expect(tip).toContainText('Running');
    await expect(tip).toContainText('Started');
    await expect(tip).toContainText('Stopped');
    await closed.dispatchEvent('pointerleave');
    await expect(tip).toHaveCount(0);

    // The legend switches runs off and on.
    const toggle = legend.getByTestId('runs-toggle');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await expect(runs).toHaveCount(0);
    await toggle.click();
    await expect(runs.first()).toBeVisible();

    // Clicking a run filters to its lane and time range.
    const all = await initialTotal(page);
    await closed.dispatchEvent('click');
    await expect(gridTotal(page)).not.toHaveText(all);
    await expect(page.getByRole('button', { name: 'Fit' })).toBeEnabled();
  });

  test('a run rule from a preset in the guided editor shows bands and fills the Runs view', async ({ page }, testInfo) => {
    await runCommand(page, 'Manage markers');
    const editor = page.locator('imt-runs-editor');
    await editor.getByTestId('runs-new').click();
    const form = editor.getByTestId('runs-form');
    await form.getByLabel('Process start / exit by process id').check();
    await expect(form.getByLabel('Name')).toHaveValue('Process');
    await expect(form.getByRole('combobox', { name: 'Pair starts and stops within' })).toHaveValue('ProcessId');
    const preview = editor.getByTestId('runs-preview');
    await expect(preview).toContainText('runs on');
    await expect(preview.getByRole('table', { name: 'First runs' })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'runs-editor', 'imt-runs-editor');

    // An invalid pattern is explained next to the field and blocks saving.
    const startPattern = form.getByRole('textbox', { name: 'Text that starts a run (regex)' });
    const original = await startPattern.inputValue();
    await startPattern.fill('(');
    await expect(form.getByRole('alert').first()).toContainText('not a valid regex');
    await expect(form.getByTestId('runs-save')).toBeDisabled();
    await startPattern.fill(original);
    await expect(preview).toContainText('runs on');
    await form.getByTestId('runs-save').click();
    await expect(form).toHaveCount(0);
    await expect(editor.locator('.rules li[data-kind="process"]')).toContainText('Process');

    // The timeline draws the new kind, with its own legend entry.
    const timeline = page.locator('imt-timeline-panel');
    await expect(timeline.getByRole('group', { name: 'Runs' }).locator('[data-run-kind="process"]')).toBeVisible();
    await expect(timeline.locator('.lane .run[data-kind="process"]').first()).toBeAttached();

    // The Runs view lists them, sortable; a row jumps to its run.
    const all = await initialTotal(page);
    await runCommand(page, 'Show runs');
    const view = page.locator('imt-runs-view');
    const table = view.getByRole('table', { name: 'Runs' });
    await expect(table.locator('tbody tr').first()).toBeVisible();
    await expect(view.locator('.card[data-kind="process"]')).toContainText('Process');
    await view.getByRole('button', { name: /^Duration/ }).click();
    await expect(table.getByRole('columnheader', { name: /Duration/ })).toHaveAttribute('aria-sort', 'descending');
    await expectNoSeriousViolations(page, testInfo, 'runs-view', 'imt-runs-view');
    await table.locator('tbody .link').first().click();
    await expect(gridTotal(page)).not.toHaveText(all);
  });
});
