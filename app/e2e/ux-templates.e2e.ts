import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';
import { showProfileEditor } from './menu-support';

test.describe('templates gallery', () => {
  test('previews a template and uses it as the profile', async ({ page }, testInfo) => {
    await openApp(page);
    await showProfileEditor(page);
    const editor = page.locator('imt-profile-editor');
    await expect(editor.locator('.bar .good')).toBeVisible();
    await editor.getByRole('button', { name: 'New from template…' }).click();
    const dialog = page.getByRole('dialog', { name: 'New profile from a template' });
    await expect(dialog.getByRole('navigation', { name: 'Templates' })).toContainText('Web servers');
    await dialog.getByRole('button', { name: 'NLog default layout' }).click();
    await expect(dialog.getByRole('button', { name: 'NLog default layout' })).toHaveAttribute('aria-pressed', 'true');
    await expect(dialog.getByTestId('template-preview-status')).toContainText('Reads 2 events');
    await expect(dialog.getByRole('table', { name: 'Preview of the parsed events' })).toContainText('Payment failed');
    await expectNoSeriousViolations(page, testInfo, 'templates-gallery', 'imt-templates-gallery');

    await dialog.getByRole('button', { name: 'Use this' }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('status').filter({ hasText: 'Applying the NLog default layout template' })).toBeVisible();
    await expect(editor.locator('.bar .name')).toHaveText('NLog default layout');
  });

  test('is offered for an unrecognised file, previewing against it', async ({ page }) => {
    await openApp(page);
    await page.locator('imt-sidebar').getByRole('button', { name: /^Set up format for/ }).first().click();
    await page.getByTestId('start-from-template').click();
    const dialog = page.getByRole('dialog', { name: 'New profile from a template' });
    await expect(dialog).toContainText('The preview shows how each template reads');
    await expect(dialog.locator('.tpl .badge').first()).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toHaveCount(0);
  });
});
