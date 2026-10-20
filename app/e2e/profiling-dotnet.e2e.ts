import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { heap, hotspots, openProfiles, runCommand } from './profiling-support';

test.describe('.NET profiling (mock plugin tool)', () => {
  test('profile a .NET process: pick, record, stop, the capture opens; framework processes ask for elevation', async ({ page }, testInfo) => {
    await openProfiles(page);
    await runCommand(page, 'Profile .NET process');
    const dialog = page.getByRole('dialog', { name: 'Profile .NET process' });
    const list = dialog.getByRole('listbox', { name: 'NET processes' });
    await expect(list.getByRole('option')).toHaveCount(3);
    await expectNoSeriousViolations(page, testInfo, 'profiling-dotnet-dialog', 'imt-dotnet-dialog');

    await list.getByRole('option', { name: /Legacy\.Billing/ }).click();
    await expect(dialog.getByRole('alert').first()).toContainText('administrator');

    await list.getByRole('option', { name: /Shop\.Web/ }).click();
    await dialog.getByRole('radio', { name: /Allocations/ }).check();
    await dialog.getByTestId('dotnet-record').click();
    await expect(dialog.locator('li.cap[data-state="recording"]')).toBeVisible();
    await dialog.getByRole('button', { name: /Stop recording Shop\.Web/ }).click();
    await expect(dialog.locator('li.cap[data-state="done"]')).toBeVisible();
    await dialog.getByRole('button', { name: 'Close' }).last().click();
    await expect(hotspots(page)).toBeVisible();
  });

  test('a heap snapshot ends by itself and opens on its findings', async ({ page }) => {
    await openProfiles(page);
    await runCommand(page, 'Profile .NET process');
    const dialog = page.getByRole('dialog', { name: 'Profile .NET process' });
    await dialog.getByRole('radio', { name: /Heap snapshot/ }).check();
    await dialog.getByTestId('dotnet-record').click();
    await expect(dialog.locator('li.cap[data-state="done"]')).toBeVisible({ timeout: 10_000 });
    await dialog.getByRole('button', { name: 'Close' }).last().click();
    await expect(heap(page).getByRole('list', { name: 'Memory findings' })).toBeVisible();
  });

  test('open a process from an .etl trace', async ({ page }) => {
    await openProfiles(page);
    await runCommand(page, 'Open process from .etl');
    const dialog = page.getByRole('dialog', { name: 'Open a process from an ETW trace' });
    await expect(dialog.getByRole('listbox', { name: 'Processes in the trace' }).getByRole('option')).toHaveCount(3);
    await dialog.getByTestId('etl-open').click();
    await expect(dialog).toHaveCount(0);
    await expect(hotspots(page)).toBeVisible();
  });
});
