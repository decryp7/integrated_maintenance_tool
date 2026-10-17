import { expect, Page, test } from '@playwright/test';
import { initialTotal, openApp } from './app';
import { showProfileEditor } from './menu-support';

async function openProfile(page: Page) {
  await openApp(page);
  await showProfileEditor(page);
  await expect(page.locator('imt-profile-editor')).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Form', exact: true })).toHaveAttribute('aria-selected', 'true');
  // The profile loads asynchronously; edits made before it arrives would be overwritten.
  await expect(page.locator('imt-profile-editor .bar .name')).toHaveText(/\(Serilog\)/);
  await expect(page.locator('imt-profile-editor .bar .good')).toBeVisible();
}

test.describe('profile editor', () => {
  test('switches to the TOML view and back', async ({ page }) => {
    await openProfile(page);
    await page.getByRole('tab', { name: 'TOML', exact: true }).click();
    const toml = page.getByRole('textbox', { name: 'Profile TOML' });
    await expect(toml).toBeVisible();
    await expect(toml).toHaveValue(/\[parse\]/);
    await expect(page.locator('imt-profile-form')).toHaveCount(0);

    await page.getByRole('tab', { name: 'Form', exact: true }).click();
    await expect(page.locator('imt-profile-form')).toBeVisible();
    await expect(toml).toHaveCount(0);
  });

  test('editing an extractor regex updates the live preview highlights', async ({ page }) => {
    await openProfile(page);
    await page.getByRole('navigation', { name: 'Profile sections' }).getByRole('button', { name: /^Extractors/ }).click();
    const editor = page.locator('imt-extractor-editor');
    await expect(editor).toBeVisible();
    await expect(editor.locator('.stat')).toBeVisible();

    await editor.getByLabel(/^Pattern/).fill('(?P<marker_word>Process)');
    await expect(editor.locator('.hl[title="marker_word"]').first()).toHaveText('Process');
    await expect(editor.locator('.stat')).toContainText(/\d+ matches/);
    await expect(page.getByText('● Unsaved')).toBeVisible();
  });

  test('invalid TOML shows an error with a line number', async ({ page }) => {
    await openProfile(page);
    await page.getByRole('tab', { name: 'TOML', exact: true }).click();
    const toml = page.getByRole('textbox', { name: 'Profile TOML' });
    await toml.press('ControlOrMeta+Home');
    await page.keyboard.type('this is = = not toml\n');

    const problem = page.locator('imt-toml-editor .problem');
    await expect(problem).toBeVisible();
    await expect(problem.getByRole('button', { name: /^Line \d+$/ })).toBeVisible();
    await expect(page.locator('imt-profile-editor .bar .bad')).toContainText(/Line \d+:/);
    await expect(page.getByRole('button', { name: 'Save & apply' })).toBeDisabled();
  });

  test('saves a change and re-applies the profile', async ({ page }) => {
    await openApp(page);
    const all = await initialTotal(page);
    await openProfile(page);
    await page.getByRole('navigation', { name: 'Profile sections' }).getByRole('button', { name: /^General/ }).click();
    await page.getByLabel('Profile name').fill('E2E profile');
    await expect(page.getByText('● Unsaved')).toBeVisible();

    const save = page.getByRole('button', { name: 'Save & apply' });
    await expect(save).toBeEnabled();
    await save.click();
    await expect(page.getByText('● Unsaved')).toHaveCount(0);
    await expect(save).toBeDisabled();
    await expect(page.locator('imt-profile-editor .bar .name')).toHaveText('E2E profile');

    // The workspace was re-imported with the saved profile.
    await page.getByRole('tab', { name: /^Timeline/ }).click();
    await expect(page.locator('imt-events-grid .bar .total')).toHaveText(all);
  });
});
