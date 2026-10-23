import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith } from './mock-url';

// AI runtime and model update checks against the scripted AI mock (src/app/core/mock/mock-ai-updates.ts):
// the "Update / Later / Skip this version" message, and Settings → AI → Updates.

const settingsDialog = (page: Page) => page.getByRole('dialog', { name: 'Settings' });
const KEY = 'runtime:vulkan@b11700';

async function openAiSettings(page: Page) {
  await page.getByRole('button', { name: 'Settings' }).click();
  const dialog = settingsDialog(page);
  await dialog.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: /AI assistant/ }).click();
  const updates = dialog.getByTestId('ai-updates');
  await expect(updates).toBeVisible();
  return updates;
}

test('an update found by the check is announced with Update, Later and Skip this version', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockAiUpdates=available&mockAiStepMs=5');
  const toast = page.getByTestId('notification').filter({ hasText: 'AI runtime update' });
  await expect(toast).toBeVisible();
  await expect(toast).toContainText('b11700 replaces b11669');
  await expect(toast.getByTestId('notification-action-primary')).toHaveText('Update');
  await expect(toast.getByTestId('notification-action-secondary')).toHaveText('Later');
  await expect(toast.getByTestId('notification-action-tertiary')).toHaveText('Skip this version');
  await expectNoSeriousViolations(page, testInfo, 'ai-update-toast', '[data-testid=notification]');

  // Update runs as a download job and the message goes.
  await toast.getByTestId('notification-action-primary').click();
  await expect(page.locator('debrief-jobs-panel')).toContainText('Updating the Vulkan (any GPU vendor) inference runtime');
  await expect(toast).toHaveCount(0);

  // Once installed, Settings has nothing to offer for the runtime.
  const updates = await openAiSettings(page);
  await expect(updates.getByTestId(`ai-update-${KEY}`)).toHaveCount(0, { timeout: 15_000 });
  await expect(updates.getByTestId('ai-updates-current-runtime')).toContainText('Up to date');
});

test('Settings lists updates with Check now and remembers Skip this version', async ({ page }, testInfo) => {
  await openAppWith(page, '?mockAiUpdates=manual&mockAiStepMs=5');
  const updates = await openAiSettings(page);
  const runtime = updates.getByTestId('ai-updates-runtime');
  await expect(runtime).toContainText('Never checked');
  await expect(updates.getByTestId('ai-updates-policy-runtime')).toHaveText('Manual');
  await expect(runtime).toContainText('Checks only when you choose Check now');
  await expect(page.getByTestId('notification').filter({ hasText: 'AI runtime update' })).toHaveCount(0);

  await updates.getByTestId('ai-updates-check').click();
  const row = updates.getByTestId(`ai-update-${KEY}`);
  await expect(row).toContainText('b11669 → b11700');
  await expect(runtime).toContainText('Checked');
  await expect(page.getByTestId('notification').filter({ hasText: 'AI runtime update' })).toHaveCount(0);
  await expectNoSeriousViolations(page, testInfo, 'ai-updates-settings', '[role=dialog]');

  await row.getByRole('button', { name: 'Skip this version of Vulkan (any GPU vendor) (b11700)' }).click();
  await expect(row).toContainText('Skipped');
  await expect(row.getByTestId('ai-update-skip')).toHaveCount(0);
  await row.getByRole('button', { name: 'Update Vulkan (any GPU vendor) to b11700' }).click();
  await expect(row).toContainText('Installing');
});

test('a check blocked by a TLS-inspecting proxy is quiet and offers the manual download', async ({ page }) => {
  await openAppWith(page, '?mockAiUpdates=manual&mockAiNet=tls&mockAiStepMs=5');
  const updates = await openAiSettings(page);
  await updates.getByTestId('ai-updates-check').click();
  await expect(updates.getByTestId('ai-updates-failed-runtime')).toContainText('Zscaler');
  await expect(page.getByTestId('notification').filter({ hasText: 'AI runtime update' })).toHaveCount(0);
});
