import { expect, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand } from './tools-support';

test.describe('lifecycle banners and elevation', () => {
  test('safe mode shows a banner with Restart normally', async ({ page }, testInfo) => {
    await openAppWith(page, '?mock-safe=1');
    const banner = page.getByTestId('safe-mode-banner');
    await expect(banner).toContainText('Running in safe mode: plugins and saved profile are off');
    // Part of the shell: the banners sit right below the title bar, the project name inside it.
    await expect(page.locator('imt-title-bar + imt-lifecycle-banners')).toHaveCount(1);
    await expect(page.locator('imt-title-bar').getByTestId('project-title')).toContainText('Untitled');
    await expectNoSeriousViolations(page, testInfo, 'safe-mode-banner');
    // Save diagnostics… asks where to save (the browser build takes the suggested name); a job writes the zip and its
    // message reports the result.
    await banner.getByTestId('save-diagnostics').click();
    await expect(page.getByTestId('diagnostics-export-status')).toContainText(/Saving diagnostics to imt-diagnostics-\d{8}-\d{6}\.zip/);
    const saved = page.getByTestId('notice-toast').filter({ hasText: /Finished: .*imt-diagnostics-\d{8}-\d{6}\.zip/ });
    await expect(saved).toContainText('files zipped.', { timeout: 15_000 });
    await page.getByTestId('diagnostics-export-status').getByRole('button', { name: 'Dismiss' }).click();
    await banner.getByRole('button', { name: 'Restart normally' }).click();
    await expect(page.getByRole('alert').filter({ hasText: 'Could not restart' })).toHaveCount(0);
  });

  test('a previous crash offers restore, safe mode and dismiss', async ({ page }, testInfo) => {
    await openAppWith(page, '?mock-crash=1');
    const banner = page.getByTestId('crash-banner');
    await expect(banner).toContainText("IMT didn't close properly last time");
    await expect(banner.getByRole('button', { name: 'Start in safe mode' })).toBeVisible();
    await expect(banner.getByTestId('save-diagnostics')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'crash-banner');
    await banner.getByRole('button', { name: 'Restore last session' }).click();
    await expect(banner).toHaveCount(0);

    await openAppWith(page, '?mock-crash=1');
    await page.getByTestId('crash-banner').getByRole('button', { name: 'Dismiss' }).click();
    await expect(page.getByTestId('crash-banner')).toHaveCount(0);
  });

  test('no banners on a normal start', async ({ page }) => {
    await openAppWith(page);
    await expect(page.getByTestId('safe-mode-banner')).toHaveCount(0);
    await expect(page.getByTestId('crash-banner')).toHaveCount(0);
    await expect(page.getByTestId('admin-badge')).toHaveCount(0);
  });

  test('the Administrator badge shows when elevated and the restart command is offered only when it can work', async ({ page }) => {
    await openAppWith(page, '?mock-elevated=1');
    await expect(page.getByTestId('admin-badge')).toHaveText('Administrator');
    await page.keyboard.press('ControlOrMeta+k');
    await page.getByRole('combobox', { name: 'Type a command' }).fill('Restart as administrator');
    await expect(page.locator('imt-command-palette').getByRole('option')).toHaveCount(0);
    await page.keyboard.press('Escape');

    await openAppWith(page);
    await runCommand(page, 'Restart as administrator');
    await expect(page.getByRole('alert').filter({ hasText: 'Could not restart' })).toHaveCount(0);

    for (const os of ['mac', 'linux']) {
      await openAppWith(page, `?mock-os=${os}`);
      await page.keyboard.press('ControlOrMeta+k');
      await page.getByRole('combobox', { name: 'Type a command' }).fill('Restart as administrator');
      await expect(page.locator('imt-command-palette').getByRole('option')).toHaveCount(0);
      await page.keyboard.press('Escape');
    }
  });
});
