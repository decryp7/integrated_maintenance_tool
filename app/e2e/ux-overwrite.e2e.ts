import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, openView, runCommand } from './tools-support';
import { chooseMenu } from './menu-support';

async function exportEvents(page: Page) {
  await page.keyboard.press('ControlOrMeta+Shift+e');
  const dialog = page.getByRole('dialog', { name: 'Export events' });
  await expect(dialog.getByRole('status')).toContainText('events will be exported');
  await dialog.getByRole('button', { name: 'Export…' }).click();
  return dialog;
}

test('an export never replaces an existing file without asking', async ({ page }, testInfo) => {
  await openApp(page);
  // First export: a new file, no question.
  let dialog = await exportEvents(page);
  await expect(dialog).toHaveCount(0);
  const done = page.getByTestId('notice-toast').filter({ hasText: 'Finished: Export events to events.csv' });
  await expect(done).toBeVisible({ timeout: 15_000 });
  await page.getByTestId('notice-toast').filter({ hasText: 'running in Jobs' }).getByRole('button', { name: 'Dismiss notification' }).click();
  await done.getByRole('button', { name: 'Dismiss notification' }).click();

  // Same file again: the engine refuses, the app asks.
  dialog = await exportEvents(page);
  const confirm = page.getByRole('dialog', { name: 'Replace file?' });
  await expect(confirm).toContainText('events.csv already exists');
  await expectNoSeriousViolations(page, testInfo, 'overwrite-confirm');
  await confirm.getByRole('button', { name: 'Keep the existing file' }).click();
  await expect(confirm).toHaveCount(0);
  // Nothing was written and the export dialog is still there to choose another name.
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId('notice-toast').filter({ hasText: 'Exporting to events.csv' })).toHaveCount(0);

  await dialog.getByRole('button', { name: 'Export…' }).click();
  await page.getByRole('dialog', { name: 'Replace file?' }).getByRole('button', { name: 'Replace' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('notice-toast').filter({ hasText: 'Exporting to events.csv — running in Jobs' })).toBeVisible();
});

test('About says Debrief never modifies your files', async ({ page }) => {
  await openApp(page);
  await chooseMenu(page, 'Help', 'About Debrief');
  await expect(page.getByTestId('read-only-promise')).toContainText('Debrief never modifies your files.');
});

test.describe('tools never replace a file without asking', () => {
  test('a diagnostics bundle of the same name', async ({ page }) => {
    await openAppWith(page);
    await runCommand(page, 'Collect diagnostics bundle');
    const dialog = page.getByRole('dialog', { name: 'Collect diagnostics bundle' });
    await dialog.getByLabel('Output folder').fill('/demo/out');
    await dialog.getByRole('button', { name: 'Collect', exact: true }).click();
    const result = dialog.getByTestId('collect-result');
    await expect(result).toContainText('/demo/out/debrief-diagnostics.debriefz');

    // The same bundle name again: the app asks, and "Keep" collects nothing.
    await dialog.getByRole('button', { name: 'Collect again' }).click();
    const confirm = page.getByRole('dialog', { name: 'Replace file?' });
    await expect(confirm).toContainText('debrief-diagnostics.debriefz already exists');
    await confirm.getByRole('button', { name: 'Keep the existing file' }).click();
    await expect(confirm).toHaveCount(0);
    await expect(result).toHaveCount(0);
    await expect(dialog.getByRole('alert')).toHaveCount(0);

    await dialog.getByRole('button', { name: 'Collect', exact: true }).click();
    await page.getByRole('dialog', { name: 'Replace file?' }).getByRole('button', { name: 'Replace' }).click();
    await expect(result).toContainText('/demo/out/debrief-diagnostics.debriefz');
  });

  test('a Performance Monitor template saved twice', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Performance Monitor');
    const panel = page.locator('debrief-perfmon-view');
    await panel.getByRole('textbox', { name: 'Collector name' }).fill('Repro');
    await panel.getByRole('button', { name: 'Generate XML' }).click();
    await panel.getByRole('button', { name: 'Save as…' }).click();
    await expect(panel.getByRole('status').filter({ hasText: 'Saved Repro.xml' })).toBeVisible();

    await panel.getByRole('button', { name: 'Save as…' }).click();
    const confirm = page.getByRole('dialog', { name: 'Replace file?' });
    await expect(confirm).toContainText('Repro.xml already exists');
    await confirm.getByRole('button', { name: 'Keep the existing file' }).click();
    await expect(panel.getByRole('status').filter({ hasText: 'Saved Repro.xml' })).toHaveCount(0);

    await panel.getByRole('button', { name: 'Save as…' }).click();
    await page.getByRole('dialog', { name: 'Replace file?' }).getByRole('button', { name: 'Replace' }).click();
    await expect(panel.getByRole('status').filter({ hasText: 'Saved Repro.xml' })).toBeVisible();
  });

  test('the agent configuration saved over another existing file', async ({ page }) => {
    await openAppWith(page);
    await openView(page, 'Collection schedules');
    const panel = page.locator('debrief-schedules-view');
    await expect(panel.getByTestId('schedules-state')).toHaveText('Saved');
    const path = panel.getByRole('textbox', { name: 'Config file' });
    const save = panel.getByRole('button', { name: 'Save' });

    // Its own file: saved without a question.
    await save.click();
    await expect(panel.getByRole('status').first()).toContainText('Saved /demo/config/debrief-agent.toml');
    // A new file: no question either.
    await path.fill('/demo/config/copy.toml');
    await save.click();
    await expect(panel.getByRole('status').first()).toContainText('Saved /demo/config/copy.toml');
    await expect(page.getByRole('dialog', { name: 'Replace file?' })).toHaveCount(0);

    // Back to the first file, which this editor did not load: asked before replacing it.
    await path.fill('/demo/config/debrief-agent.toml');
    await save.click();
    const confirm = page.getByRole('dialog', { name: 'Replace file?' });
    await expect(confirm).toContainText('debrief-agent.toml already exists');
    await confirm.getByRole('button', { name: 'Keep the existing file' }).click();
    await expect(panel.getByRole('status').first()).toContainText('Not saved: the existing file was kept.');
    await save.click();
    await page.getByRole('dialog', { name: 'Replace file?' }).getByRole('button', { name: 'Replace' }).click();
    await expect(panel.getByRole('status').first()).toContainText('Saved /demo/config/debrief-agent.toml');
  });
});
