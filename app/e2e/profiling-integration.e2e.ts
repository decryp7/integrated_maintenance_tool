import { expect, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';
import { expectNoSeriousViolations } from './axe';
import { hotspots, openProfiles, runCommand, showCaptures } from './profiling-support';

test.describe('Profiling with the logs', () => {
  test('capture spans on the log timeline open the profile', async ({ page }, testInfo) => {
    await openProfiles(page);
    // Opening a capture switched to the Profiling perspective; the log timeline is in Investigate.
    await page.keyboard.press('ControlOrMeta+1');
    await page.getByRole('tab', { name: /Timeline/ }).click();
    await runCommand(page, 'Profiling: show captures');
    const spans = page.locator('imt-timeline-panel').getByTestId('profile-span');
    await expect(spans).toHaveCount(5);
    await expectNoSeriousViolations(page, testInfo, 'profiling-captures', 'imt-captures-view');

    // Make the other profile active, then open the first from the timeline.
    await (await showCaptures(page)).getByRole('button', { name: 'orders-regression.speedscope.json', exact: true }).click();
    await page.getByRole('tab', { name: /Timeline/ }).click();
    await page.locator('imt-timeline-panel').getByRole('button', { name: 'Open capture orders.speedscope.json' }).click();
    await expect(hotspots(page).getByRole('combobox', { name: 'Active profile' }).locator('option:checked')).toHaveText('orders.speedscope.json');

    await page.getByRole('tab', { name: /Timeline/ }).click();
    await page.locator('imt-timeline-panel').getByRole('button', { name: 'Open heap snapshot heap-2.heapsnapshot' }).click();
    await expect(page.locator('imt-heap-view').getByRole('combobox', { name: 'Heap snapshot' }).locator('option:checked')).toHaveText('heap-2.heapsnapshot');
  });

  test('the Moment view lists profile samples in the window and opens the profile on that range', async ({ page }) => {
    await openProfiles(page);
    await runCommand(page, 'Analyse this moment');
    const moment = page.locator('imt-moment-view');
    const section = moment.locator('imt-profile-moment');
    await expect(section).toContainText('Profile samples in this window');
    await expect(section.getByRole('region', { name: 'Profile orders.speedscope.json' })).toContainText('Shop.Orders.OrderService.Place');
    await section.getByRole('region', { name: 'Profile orders.speedscope.json' }).getByRole('button', { name: 'Open capture on this range' }).click();
    await expect(hotspots(page).getByTestId('range-pill')).toBeVisible();
  });

  test('a failed query shows an inline error with Retry', async ({ page }) => {
    await openProfilesWithFailure(page);
    const view = hotspots(page);
    await expect(view.getByRole('alert')).toContainText('hotspots failed');
    await view.getByRole('button', { name: 'Retry' }).click();
    await expect(view.locator('tbody td.name button').first()).toBeVisible();
  });
});

async function openProfilesWithFailure(page: import('@playwright/test').Page): Promise<void> {
  await page.goto('/?mockProfilingFail=hotspots:1');
  await expect(page.locator('imt-status-bar')).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
  await expect(page.locator('imt-status-bar')).toContainText('events');
  await runCommand(page, 'Open performance capture');
  await expect(hotspots(page)).toBeVisible();
}
