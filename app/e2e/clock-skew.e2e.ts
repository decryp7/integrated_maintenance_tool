import { expect, Page, test } from '@playwright/test';
import { expectNoSeriousViolations } from './axe';
import { openAppWith, runCommand } from './tools-support';

/** The sidebar row of a source, by the end of its path. */
function sourceRow(page: Page, path: string) {
  return page.locator('imt-sidebar .src', { hasText: path });
}

/** Searches the messages and selects the first matching event. */
async function selectFirst(page: Page, text: string): Promise<void> {
  await page.getByTestId('query-field').fill(text);
  const first = page.locator('imt-events-grid [role=row].row').first();
  await expect(first).toContainText(text);
  await first.click();
  await expect(page.locator('imt-inspector .message')).toContainText(text);
}

test.describe('clock-skew correction', () => {
  test('a manual offset shifts the source, shows a badge and the original time in the inspector', async ({ page }, testInfo) => {
    await openAppWith(page, '?mockMemoryFeatures=1');
    const row = sourceRow(page, 'gateway/gateway-20261014.json');
    await row.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Adjust clock…' }).click();
    const dialog = page.getByRole('dialog', { name: /Adjust clock/ });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId('current-offset')).toHaveText('0 s');
    const input = dialog.getByRole('textbox', { name: /Offset/ });
    await input.fill('nonsense');
    await expect(dialog.getByText('Not an offset')).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Apply', exact: true })).toBeDisabled();
    await input.fill('+5');
    await expect(dialog.getByText('Will shift by +5 s')).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'clock-dialog', 'imt-clock-dialog');
    await dialog.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(row.getByTestId('skew-badge')).toHaveText('+5 s');

    // An event of the shifted source says by how much and when it was logged originally.
    await selectFirst(page, 'Proxying');
    await expect(page.getByTestId('skew-note')).toContainText('Shifted by +5 s · original');
    await expectNoSeriousViolations(page, testInfo, 'skew-inspector', 'imt-inspector');

    // Reset from the keyboard-reachable button on the row.
    await row.hover();
    await row.getByRole('button', { name: /Adjust clock of/ }).click();
    await page.getByRole('dialog', { name: /Adjust clock/ }).getByRole('button', { name: 'Reset to 0' }).click();
    await expect(row.getByTestId('skew-badge')).toHaveCount(0);
  });

  test('aligns a source on a reference event of another source', async ({ page }) => {
    await openAppWith(page, '?mockMemoryFeatures=1');
    await selectFirst(page, 'Proxying');
    await page.locator('imt-inspector').getByRole('button', { name: 'Use as clock reference' }).click();
    await expect(page.getByTestId('is-reference')).toBeVisible();
    await selectFirst(page, 'placed by customer');
    const align = page.locator('imt-inspector').getByRole('button', { name: /^Align .* on reference…$/ });
    await align.click();
    const dialog = page.getByRole('dialog', { name: /Adjust clock/ });
    await expect(dialog.getByText('Same moment in this source')).toBeVisible();
    await dialog.getByRole('button', { name: 'Align on these events' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId('skew-badge')).toHaveCount(1);
  });

  test('suggests an offset from shared correlation ids', async ({ page }, testInfo) => {
    await openAppWith(page, '?mockMemoryFeatures=1');
    const row = sourceRow(page, 'web02/orders-20261014.clef');
    await row.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Adjust clock…' }).click();
    const dialog = page.getByRole('dialog', { name: /Adjust clock/ });
    await dialog.getByRole('textbox', { name: /Offset/ }).fill('90');
    await dialog.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(row.getByTestId('skew-badge')).toHaveText('+1 min 30 s');

    await row.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Adjust clock…' }).click();
    await dialog.getByRole('button', { name: 'Suggest' }).click();
    // The demo data is fixed, and too few of web02's events share a correlation id with another source: the answer
    // is always this explanation, never a suggestion (the old `if (isVisible)` branch never ran).
    await expect(dialog.getByText(/Too few events share a CorrelationId/)).toBeVisible();
    await expect(dialog.getByTestId('suggestion')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'clock-suggestion', 'imt-clock-dialog');
  });

  test('palette commands work on the selected event', async ({ page }) => {
    await openAppWith(page, '?mockMemoryFeatures=1');
    await page.locator('imt-events-grid [role=row].row').nth(3).click();
    await runCommand(page, 'Use selected event as clock reference');
    await expect(page.getByTestId('is-reference')).toBeVisible();
    await runCommand(page, "Adjust clock of the selected event's source");
    await expect(page.getByRole('dialog', { name: /Adjust clock/ })).toBeVisible();
  });
});
