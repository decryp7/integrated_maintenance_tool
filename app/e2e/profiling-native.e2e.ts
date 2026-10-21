import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { findings, hotspots, openProfiles, runCommand } from './profiling-support';

test.describe('Native profiling (mock plugin tools)', () => {
  test('sample a native process: pick, record, stop, the capture opens on its findings; ETW-only processes say so', async ({ page }, testInfo) => {
    await openProfiles(page);
    await runCommand(page, 'Profile native process');
    const dialog = page.getByRole('dialog', { name: 'Profile native process' });
    const list = dialog.getByRole('listbox', { name: 'Processes' });
    await expect(list.getByRole('option')).toHaveCount(3);
    await expect(dialog.getByLabel('Symbol path (folders with PDB files, separated by ;)')).toHaveValue('C:\\symbols');
    await expectNoSeriousViolations(page, testInfo, 'profiling-native-dialog', 'imt-native-dialog');

    await list.getByRole('option', { name: /svchost/ }).click();
    await expect(dialog.getByRole('alert').first()).toContainText('ETW (administrator)');
    await expect(dialog.getByTestId('native-record')).toBeDisabled();

    await list.getByRole('option', { name: /OrderService/ }).click();
    await dialog.getByLabel('Samples per second per thread').fill('200');
    await dialog.getByTestId('native-record').click();
    await expect(dialog.locator('li.cap[data-state="recording"]')).toBeVisible();
    await dialog.getByRole('button', { name: /Stop recording OrderService/ }).click();
    await expect(dialog.locator('li.cap[data-state="done"]')).toBeVisible();
    await dialog.getByRole('button', { name: 'Close' }).last().click();
    await expect(findings(page)).toBeVisible();
  });

  test('ETW records with heap allocations', async ({ page }) => {
    await openProfiles(page);
    await runCommand(page, 'Profile native process');
    const dialog = page.getByRole('dialog', { name: 'Profile native process' });
    await dialog.getByRole('radio', { name: /ETW \(administrator\)/ }).check();
    await dialog.getByRole('checkbox', { name: /Heap allocations/ }).check();
    await dialog.getByTestId('native-record').click();
    await expect(dialog.locator('li.cap[data-state="recording"]')).toContainText('ETW');
  });

  test('open a process (.NET or native) from an .etl trace', async ({ page }) => {
    await openProfiles(page);
    await runCommand(page, 'Open process from .etl');
    const dialog = page.getByRole('dialog', { name: 'Open a process from an ETW trace' });
    await expect(dialog.getByRole('listbox', { name: 'Processes in the trace' }).getByRole('option')).toHaveCount(3);
    await dialog.getByTestId('etl-open').click();
    await expect(dialog).toHaveCount(0);
    await expect(hotspots(page)).toBeVisible();
  });
});
