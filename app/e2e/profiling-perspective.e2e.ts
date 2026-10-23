import { expect, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT, openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { chooseMenu } from './menu-support';
import { hotspots } from './profiling-support';

// The Captures view is the landing of the Profiling perspective; opening a performance capture switches there once
// (the toast offers Undo), and not again for the next capture.

// A workspace notice (the status toast with one action), not a Notifications entry.
const toast = (page: import('@playwright/test').Page) => page.locator('[data-toast]').filter({ hasText: 'Switched to the Profiling perspective' });

test.describe('profiling perspective', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.clear());
    await openApp(page);
  });

  test('Ctrl+2 shows the Captures landing with Open capture…', async ({ page }, testInfo) => {
    await page.keyboard.press('ControlOrMeta+2');
    const landing = page.getByTestId('captures-landing');
    await expect(landing).toBeVisible();
    await expect(landing.getByRole('button', { name: 'Open capture…' })).toBeEnabled();
    await expectNoSeriousViolations(page, testInfo, 'captures-landing', 'debrief-captures-view');
    await landing.getByRole('button', { name: 'Open capture…' }).click();
    await expect(hotspots(page).locator('tbody tr[aria-rowindex]').first()).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
    await expect(page.getByTestId('captures-landing')).toHaveCount(0);
    await expect(page.getByRole('tab', { name: 'Captures' })).toHaveCount(1);
  });

  test('opening a capture switches to Profiling once, with an Undo that goes back', async ({ page }) => {
    await expect(page.locator('debrief-timeline-panel')).toBeVisible();
    await chooseMenu(page, 'File', 'Open performance capture…');
    await expect(hotspots(page).locator('tbody tr[aria-rowindex]').first()).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
    await expect(page.locator('debrief-timeline-panel')).toHaveCount(0);
    const notice = toast(page);
    await expect(notice).toBeVisible();
    await expect(notice.getByTestId('notice-action')).toHaveText('Undo');
    await expectNoSeriousViolations(page, test.info(), 'profiling-switch-toast', '[data-toast]');
    await notice.getByTestId('notice-action').click();
    await expect(page.locator('debrief-timeline-panel')).toBeVisible();
    await expect(page.getByRole('grid', { name: 'Events' })).toBeVisible();

    // Opened again from Investigate: it does not switch a second time.
    await chooseMenu(page, 'File', 'Open performance capture…');
    await expect(page.getByRole('tab', { name: 'Captures' })).toHaveCount(1);
    await expect(page.locator('debrief-timeline-panel')).toBeVisible();
    await expect(toast(page)).toHaveCount(0);
  });

  test('already in Profiling: opening a capture does not announce a switch', async ({ page }) => {
    await page.keyboard.press('ControlOrMeta+2');
    await chooseMenu(page, 'File', 'Open performance capture…');
    await expect(hotspots(page).locator('tbody tr[aria-rowindex]').first()).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
    await expect(toast(page)).toHaveCount(0);
  });
});
