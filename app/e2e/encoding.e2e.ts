import { expect, test } from '@playwright/test';
import { openAppWith } from './tools-support';

test.describe('source encodings', () => {
  test('a guessed encoding is badged; Encoding… reads the source again with the chosen one', async ({ page }) => {
    await openAppWith(page, '');
    const row = page.locator('debrief-sidebar .src', { hasText: 'worker-20261014.log' });
    const badge = row.getByTestId('encoding-badge');
    await expect(badge).toHaveText('Shift_JIS 84%');

    await row.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Encoding…' }).click();
    const dialog = page.getByRole('dialog', { name: /Encoding of worker-20261014\.log/ });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText('The file is read again')).toBeVisible();
    await expect(dialog.getByTestId('encoding-current')).toContainText('Shift_JIS · 84% (detected)');
    await expect(dialog.getByRole('radio', { name: /Automatic/ })).toBeChecked();
    await dialog.getByRole('radio', { name: /windows-1252/ }).check();
    await dialog.getByRole('button', { name: 'Apply', exact: true }).click();
    await expect(dialog).toHaveCount(0);

    await expect(badge).toHaveText('windows-1252');
    await expect(badge).toHaveAttribute('title', 'windows-1252 (chosen)');
  });
});
