import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand, sourceCount } from './tools-support';

test.describe('Diagnostics collector dialog', () => {
  test('fills the plan, collects, shows the manifest summary and opens the bundle', async ({ page }, testInfo) => {
    await openAppWith(page);
    const before = await sourceCount(page);
    await runCommand(page, 'Collect diagnostics bundle');
    const dialog = page.getByRole('dialog', { name: 'Collect diagnostics bundle' });
    await expect(dialog).toBeVisible();

    await dialog.getByRole('button', { name: /Add log folder/ }).click();
    await dialog.getByRole('textbox', { name: 'Log folder 1' }).fill('C:\\ProgramData\\Contoso\\Logs');
    await dialog.getByLabel('Max age 1').fill('3');
    await dialog.getByRole('button', { name: /Add custom pattern/ }).click();
    await dialog.getByLabel('Pattern name 1').fill('order');
    await dialog.getByLabel('Pattern regex 1').fill('ORD-\\d+');
    await expectNoSeriousViolations(page, testInfo, 'collector-dialog', '[role=dialog]');

    await dialog.getByRole('button', { name: 'Edit as JSON' }).click();
    const json = dialog.getByLabel('Plan JSON');
    await expect(json).toHaveValue(/"path": "C:\\\\ProgramData\\\\Contoso\\\\Logs"/);
    await json.fill('{ nope');
    await expect(dialog.getByRole('alert')).toContainText('Not valid JSON');
    await expect(dialog.getByRole('button', { name: 'Collect', exact: true })).toBeDisabled();
    await json.fill('{ "name": "case-42" }');
    await dialog.getByRole('button', { name: 'Edit as JSON' }).click();
    await expect(dialog.getByLabel('Bundle name')).toHaveValue('case-42');

    await dialog.getByRole('button', { name: 'Collect', exact: true }).click();
    await expect(dialog.getByText('Choose an output folder')).toBeVisible();
    await dialog.getByLabel('Output folder').fill('/demo/out');
    await dialog.getByRole('button', { name: 'Collect', exact: true }).click();
    const result = dialog.getByTestId('collect-result');
    await expect(result).toContainText('/demo/out/case-42.imtz');
    await expect(result).toContainText('2 items');
    await expect(result).toContainText('3 values masked');
    await expect(result).toContainText('msinfo32: disabled in the plan');
    await dialog.getByRole('button', { name: 'Open bundle' }).click();
    await expect(dialog).toHaveCount(0);
    await expect.poll(() => sourceCount(page)).toBe(before + 1);
  });
});
