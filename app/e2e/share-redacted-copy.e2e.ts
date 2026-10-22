import { expect, test } from '@playwright/test';
import { chooseMenu } from './menu-support';
import { openAppWith } from './tools-support';

test.describe('share a redacted copy', () => {
  test('File > Share redacted copy... previews, follows the picks and creates the copy', async ({ page }) => {
    await openAppWith(page, '');
    await chooseMenu(page, 'File', 'Share redacted copy…');
    const dialog = page.getByRole('dialog', { name: 'Share redacted copy' });
    await expect(dialog).toBeVisible();

    const scanned = dialog.getByTestId('rc-scanned');
    await expect(scanned).toContainText('Scanned');
    const email = dialog.locator('[data-detector=email]');
    await expect(email).toBeChecked();
    await expect(dialog.getByTestId('rc-excluded')).toContainText('Left out of the copy');
    await expect(dialog.getByTestId('rc-notes')).toContainText('video frames');

    const before = await dialog.locator('[data-rule-hit=email]').innerText();
    await email.uncheck();
    await expect(dialog.locator('[data-rule-hit=email]')).toHaveCount(0);
    expect(before).toMatch(/Email addresses/);

    // Nothing picked: the engine's refusal shows and the action is disabled.
    const count = await dialog.locator('[data-detector]').count();
    for (let i = 0; i < count; i++) await dialog.locator('[data-detector]').nth(i).uncheck();
    await expect(dialog.getByTestId('rc-preview-error')).toBeVisible();
    await expect(dialog.getByTestId('rc-create')).toBeDisabled();

    await email.check();
    await expect(dialog.getByTestId('rc-create')).toBeEnabled();
    await dialog.getByTestId('rc-create').click();
    await expect(dialog.getByTestId('rc-result')).toContainText('Copy created');
    await expect(dialog.getByTestId('rc-result-summary')).toContainText('events written');
    await dialog.getByTestId('rc-done').click();
    await expect(dialog).toHaveCount(0);
  });
});
