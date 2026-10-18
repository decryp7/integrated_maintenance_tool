import AxeBuilder from '@axe-core/playwright';
import { expect, Page, test, TestInfo } from '@playwright/test';
import { openApp, openView } from './app';
import { WCAG_TAGS } from './axe';

/** Runs axe against WCAG 2.2 A/AA and fails on any serious or critical violation. */
async function expectNoSeriousViolations(page: Page, testInfo: TestInfo, label: string): Promise<void> {
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG_TAGS).analyze();
  const blocking = violations
    .filter((v) => v.impact === 'serious' || v.impact === 'critical')
    .map((v) => ({ id: v.id, impact: v.impact, help: v.help, count: v.nodes.length, nodes: v.nodes.slice(0, 8).map((n) => ({ target: n.target.join(' '), data: n.any[0]?.data })) }));
  if (blocking.length) console.log(`AXE ${label} ${JSON.stringify(blocking)}`);
  if (blocking.length) await testInfo.attach(`axe-${label}.json`, { body: JSON.stringify(blocking, null, 2), contentType: 'application/json' });
  expect(blocking, `axe violations (${label})`).toEqual([]);
}

for (const scheme of ['dark', 'light'] as const) {
  test(`main screen has no serious accessibility violations (${scheme})`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme: scheme });
    await openApp(page);
    await page.locator('imt-events-grid [role=row].row').first().click();
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

