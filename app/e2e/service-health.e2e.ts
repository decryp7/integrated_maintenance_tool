import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';

/** The mock engine's service health, driven from the page (the browser build; the real engine is covered in Rust). */
async function fail(page: Page, service: string, message: string, repair = 'restart', state = 'down'): Promise<void> {
  await page.waitForFunction(() => !!(globalThis as Record<string, unknown>)['__debriefMockHealth']);
  await page.evaluate(
    ([s, m, r, st]) => {
      (globalThis as unknown as { __debriefMockHealth: { fail(...a: unknown[]): void } }).__debriefMockHealth.fail(s, { message: m, hint: 'Choose Try again.', detail: 'signal: 6' }, r, st);
    },
    [service, message, repair, state],
  );
}

test.describe('service health', () => {
  test('a store failure is one chip with the cause and Repair, and no toast or panel error', async ({ page }) => {
    await openApp(page);
    const message = "Debrief's log store stopped unexpectedly.";
    for (let i = 0; i < 3; i++) await fail(page, 'store', message);
    const chip = page.getByTestId('service-health-chip');
    await expect(chip).toHaveCount(1);
    await expect(chip).toContainText('Log store: stopped');
    await expect(page.getByTestId('notification')).toHaveCount(0);
    await expect(page.locator('[data-testid=notification][data-cause="workspace.error"]')).toHaveCount(0);
    await chip.click();
    const details = page.getByTestId('service-health-details');
    await expect(details).toContainText(message);
    await expect(details).toContainText('Choose Try again.');
    await expect(details.locator('details')).not.toHaveAttribute('open', '');
    await page.getByTestId('service-health-repair-store').click();
    await expect(chip).toHaveCount(0);
  });

  test('several services are one chip with a count; each repairs on its own', async ({ page }) => {
    await openApp(page);
    await fail(page, 'ai', 'The AI worker was switched off.', 'reset', 'quarantined');
    await fail(page, 'downloads', 'A download failed: Download Qwen.', 'retry');
    const chip = page.getByTestId('service-health-chip');
    await expect(chip).toHaveText('2 services need attention');
    await chip.click();
    await page.getByTestId('service-health-repair-ai').click();
    await expect(chip).toContainText('Downloads');
    await page.getByTestId('service-health-repair-downloads').click();
    await expect(chip).toHaveCount(0);
  });
});
