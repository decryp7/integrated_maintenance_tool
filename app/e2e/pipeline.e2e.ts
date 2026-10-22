import AxeBuilder from '@axe-core/playwright';
import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { WCAG_TAGS } from './axe';
import { showProfileEditor } from './menu-support';

/** Fails on any serious or critical WCAG 2.2 A/AA violation. */
async function expectNoSeriousViolations(page: Page, label: string): Promise<void> {
  const { violations } = await new AxeBuilder({ page }).withTags(WCAG_TAGS).analyze();
  const blocking = violations
    .filter((v) => v.impact === 'serious' || v.impact === 'critical')
    .map((v) => ({ id: v.id, impact: v.impact, nodes: v.nodes.slice(0, 6).map((n) => n.target.join(' ')) }));
  expect(blocking, `axe violations (${label})`).toEqual([]);
}

test('a drop step added in the pipeline flow removes events and shows its effect', async ({ page }) => {
  await openApp(page);
  await showProfileEditor(page);
  await page.getByRole('button', { name: /^Pipeline/ }).first().click();
  await expect(page.getByText('No steps yet')).toBeVisible();

  await page.getByRole('group', { name: 'Add an activity' }).getByRole('button', { name: 'Drop' }).click();
  await expect(page.locator('.card .summary')).toHaveText('Drop events where @message contains');

  const card = page.locator('imt-pipeline-editor .card').first();
  await card.getByRole('combobox', { name: 'Field', exact: true }).fill('@level');
  await card.getByRole('textbox', { name: 'Value', exact: true }).fill('Debug');
  await expect(card.locator('.summary')).toHaveText('Drop events where @level contains Debug');
  await expect(card.locator('.stat')).toHaveText('not run yet');
  await expectNoSeriousViolations(page, 'pipeline-flow');

  await page.getByRole('button', { name: 'Save & apply' }).click();
  await expect(page.getByText('● Unsaved')).toHaveCount(0);

  await page.getByTestId('status-details').click();
  await expect(page.getByTestId('status-details-popover')).toContainText('Dropped by pipeline');
  await page.keyboard.press('Escape');
  await expect(card.locator('.stat')).toContainText(/in → .* out \(−/);
  await expect(page.locator('imt-sidebar .src[title*="dropped by pipeline"]').first()).toBeVisible();
  await expect(page.locator('imt-flow-overview')).toContainText('1 step');
});

test('steps can be reordered with the keyboard-accessible buttons and disabled', async ({ page }) => {
  await openApp(page);
  await showProfileEditor(page);
  await page.getByRole('button', { name: /^Pipeline/ }).first().click();
  const palette = page.getByRole('group', { name: 'Add an activity' });
  await palette.getByRole('button', { name: 'Drop' }).click();
  await palette.getByRole('button', { name: 'Set level' }).click();
  await page.getByRole('button', { name: 'Move step 2 up' }).click();
  await expect(page.locator('imt-pipeline-editor .card').first()).toHaveAttribute('data-step', 'set_level');
  await page.getByLabel('Disable step 1').check();
  await expect(page.locator('imt-pipeline-editor .card').first().locator('.stat')).toHaveText('disabled');
});
