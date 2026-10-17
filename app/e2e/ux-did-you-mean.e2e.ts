import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { openQuery } from './analysis-support';

test.describe('did you mean', () => {
  test('the query bar suggests the field name and fixes the query', async ({ page }, testInfo) => {
    await openApp(page);
    const input = await openQuery(page);
    await input.click();
    await input.pressSequentially('MachinName = "WEB01"');
    await expect(page.locator('#ql-error')).toContainText('unknown field "MachinName"');
    const fix = page.getByTestId('did-you-mean').getByRole('button', { name: 'Replace with MachineName' });
    await expect(fix).toBeVisible();
    await expectNoSeriousViolations(page, testInfo, 'did-you-mean', 'imt-query-view');
    await fix.click();
    await expect(input).toHaveValue('MachineName = "WEB01"');
    await expect(page.locator('#ql-error')).toHaveCount(0);
    await expect(input).toBeFocused();
  });

  test('the palette suggests close commands when nothing matches', async ({ page }) => {
    await openApp(page);
    await page.keyboard.press('ControlOrMeta+k');
    const input = page.getByRole('combobox', { name: 'Type a command' });
    await input.fill('tgogle sidbar');
    await expect(page.getByTestId('palette-did-you-mean')).toContainText('Did you mean');
    await expect(page.getByRole('option').first()).toContainText('Toggle sidebar');
    await page.keyboard.press('Enter');
    await expect(input).toHaveCount(0);
    await expect(page.locator('imt-sidebar')).toHaveCount(0);
  });
});
