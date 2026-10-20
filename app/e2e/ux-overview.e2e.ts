import { expect, test } from '@playwright/test';
import { gridTotal, initialTotal, openApp, parseCount } from './app';
import { expectNoSeriousViolations } from './axe';
import { runCommand } from './analysis-support';

test.describe('overview (health check)', () => {
  test('Focused layout (default): the headline is on the timeline header and Review opens the Overview', async ({ page }, testInfo) => {
    await openApp(page);
    const total = parseCount(await initialTotal(page));
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('msinfo32.nfo', { timeout: 10_000 });
    await expect(page.locator('imt-overview-view')).toHaveCount(0);
    const headline = page.getByRole('toolbar', { name: 'Health check' });
    await expect(headline).toContainText(/crash/);
    await expectNoSeriousViolations(page, testInfo, 'overview-headline', '[data-testid=overview-headline]');
    await headline.getByRole('button', { name: /errors?$/ }).click();
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(total);
    await headline.getByRole('button', { name: 'Review' }).click();
    await expect(page.locator('imt-overview-view')).toBeVisible();
    await expect(headline).toHaveCount(0);
  });

  test('opens after an import in the Full layout, groups findings by severity, and its actions work', async ({ page }, testInfo) => {
    await openApp(page);
    await runCommand(page, 'Layout: full');
    const total = parseCount(await initialTotal(page));
    await expect(page.locator('imt-overview-view')).toHaveCount(0);

    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    const overview = page.locator('imt-overview-view');
    await expect(overview).toBeVisible({ timeout: 10_000 });
    const region = overview.getByRole('region', { name: 'Health check' });
    await expect(region.getByText('Sources', { exact: true })).toBeVisible();
    await expect(region).toContainText('msinfo32.nfo');
    const critical = region.locator('section[data-severity=critical]');
    await expect(critical.getByRole('heading', { name: /Critical/ })).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'overview', 'imt-overview-view');

    // "Show these events" applies the finding's time range: fewer events match.
    await critical.getByRole('button', { name: 'Show these events' }).first().click();
    await expect.poll(async () => parseCount(await gridTotal(page).innerText())).toBeLessThan(total);

    // The unrecognised file's action opens the format wizard.
    await region.locator('section[data-severity=info]').getByRole('button', { name: 'Set up format…' }).first().click();
    await expect(page.getByRole('dialog', { name: 'Set up format' })).toBeVisible();
  });

  test('can be turned off for later imports', async ({ page }) => {
    await openApp(page);
    await runCommand(page, 'Layout: full');
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    const overview = page.locator('imt-overview-view');
    await expect(overview).toBeVisible({ timeout: 10_000 });
    await overview.getByRole('checkbox', { name: 'Show the overview after opening files' }).uncheck();
    // Close the panel, open more files: it stays closed.
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('Toggle Overview panel');
    await page.keyboard.press('Enter');
    await expect(overview).toHaveCount(0);
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open folder' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('extra-folder', { timeout: 10_000 });
    await expect(overview).toHaveCount(0);
  });

  test('can be turned off in General settings', async ({ page }) => {
    await openApp(page);
    await runCommand(page, 'Layout: full');
    await page.getByRole('button', { name: 'Settings' }).click();
    const dialog = page.getByRole('dialog', { name: 'Settings' });
    const auto = dialog.getByRole('checkbox', { name: /Overview: show after every import/ });
    await expect(auto).toBeChecked();
    await auto.uncheck();
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await page.locator('imt-sidebar').getByRole('button', { name: 'Open files' }).click();
    await expect(page.locator('imt-sidebar')).toContainText('msinfo32.nfo', { timeout: 10_000 });
    await expect(page.locator('imt-overview-view')).toHaveCount(0);
  });
});
