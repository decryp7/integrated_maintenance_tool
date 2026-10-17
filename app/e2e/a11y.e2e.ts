import AxeBuilder from '@axe-core/playwright';
import { expect, Page, test, TestInfo } from '@playwright/test';
import { openApp } from './app';

/** Runs axe against WCAG 2.1 A/AA and fails on any serious or critical violation. */
async function expectNoSeriousViolations(page: Page, testInfo: TestInfo, label: string): Promise<void> {
  const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
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
  await page.getByRole('tab', { name: /Metrics/ }).click();
  await expect(page.locator('imt-chart-card').first()).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'metrics');
  await page.getByRole('tab', { name: /Profile/ }).click();
  await expect(page.getByText('Parsing profile')).toBeVisible();
  await expectNoSeriousViolations(page, testInfo, 'profile');
});

test('splitter exposes its value and resizes with the keyboard', async ({ page }) => {
  await openApp(page);
  const splitter = page.getByRole('separator', { name: 'Resize timeline' });
  const before = Number(await splitter.getAttribute('aria-valuenow'));
  await splitter.focus();
  await page.keyboard.press('ArrowDown');
  await expect(splitter).toHaveAttribute('aria-valuenow', String(before + 24));
  await page.keyboard.press('Home');
  await expect(splitter).toHaveAttribute('aria-valuenow', '120');
  await expect(splitter).toHaveAttribute('aria-valuemin', '120');
});
