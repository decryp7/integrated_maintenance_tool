import { expect, test } from '@playwright/test';
import { openApp, openView } from './app';
import { expectNoSeriousViolations } from './axe';

for (const scheme of ['dark', 'light'] as const) {
  test(`main screen has no serious accessibility violations (${scheme})`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme: scheme });
    await openApp(page);
    await page.locator('imt-events-grid [role=row].row').first().click();
    // The click opens the inspector, which slides in: axe scans it once it is there and settled.
    await expect(page.locator('imt-inspector')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, `timeline-${scheme}`);
  });
}

test('metrics and profile tabs have no serious accessibility violations', async ({ page }, testInfo) => {
  await openApp(page);
  await openView(page, 'metrics');
  await expect(page.locator('imt-chart-card').first()).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'metrics');
  await page.getByRole('button', { name: 'Profile editor' }).click();
  await expect(page.getByText('Parsing profile')).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'profile');
});
