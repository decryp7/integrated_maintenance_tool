import { expect, test } from '@playwright/test';
import { openApp } from './app';
import { expectNoSeriousViolations } from './axe';

// Human-like name matching in search everything (Mod+Shift+F) against the mock backend: the "Did you mean" line says
// which name the words mean, why, how else it is written and whether an AI suggested it; accepting or rejecting is
// remembered and can be undone; two things that match equally produce a question instead of a guess.

test.describe('search names', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
    await page.locator('debrief-events-grid').first().click();
    await page.keyboard.press('ControlOrMeta+Shift+F');
  });

  const dialog = (page: import('@playwright/test').Page) => page.getByRole('dialog', { name: 'Search everything' });
  const input = (page: import('@playwright/test').Page) => dialog(page).getByRole('combobox', { name: 'Search everything' });

  test('explains the match, lists the other spellings, and remembers an accepted name until Undo', async ({ page }, testInfo) => {
    const d = dialog(page);
    await input(page).fill('graphic builder');
    const line = d.getByTestId('search-everything-suggestion');
    await expect(line).toContainText('Did you mean');
    await expect(line).toContainText('(executable)?');
    await expect(d.getByTestId('search-everything-why')).toContainText('same name written differently');
    await expect(d.getByTestId('search-everything-spellings')).toContainText('graphic-builder.exe');
    await expect(d.getByTestId('search-everything-ai')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'search-names-suggestion', 'debrief-search-everything');

    await d.getByTestId('search-everything-accept').click();
    await expect(input(page)).toHaveValue('graphicbuilder.exe');
    await expect(d.getByTestId('search-everything-feedback')).toContainText('“graphic builder” means graphicbuilder.exe');
    await d.getByTestId('search-everything-undo').click();
    await expect(input(page)).toHaveValue('graphic builder');
    await expect(d.getByTestId('search-everything-feedback')).toHaveCount(0);
  });

  test('an alias the local model suggested is labelled AI-generated', async ({ page }) => {
    await input(page).fill('image editor');
    const d = dialog(page);
    await expect(d.getByTestId('search-everything-suggestion')).toContainText('graphicbuilder.exe');
    await expect(d.getByTestId('search-everything-ai')).toContainText('AI-generated');
    await expect(d.getByTestId('search-everything-why')).toContainText('AI alias');
  });

  test('a rejected match is not offered again for those words', async ({ page }) => {
    const d = dialog(page);
    await input(page).fill('graphic builder');
    await d.getByTestId('search-everything-reject').click();
    await expect(d.getByTestId('search-everything-feedback')).toContainText('is not graphicbuilder.exe');
    await expect(d.getByTestId('search-everything-suggestion')).toHaveCount(0);
    await d.getByTestId('search-everything-undo').click();
    await expect(d.getByTestId('search-everything-suggestion')).toContainText('Did you mean');
  });

  test('two things that match equally produce a question, and the answer is remembered', async ({ page }, testInfo) => {
    const d = dialog(page);
    await input(page).fill('builder');
    const ask = d.getByTestId('search-everything-clarify');
    await expect(ask).toContainText('Which one do you mean: Graphic Builder or Report Builder?');
    await expect(d.getByTestId('search-everything-suggestion')).toHaveCount(0);
    await expectNoSeriousViolations(page, testInfo, 'search-names-clarify', 'debrief-search-everything');
    await ask.getByTestId('search-everything-option').filter({ hasText: 'CompanyA.Report.Builder.exe' }).click();
    await expect(input(page)).toHaveValue('CompanyA.Report.Builder.exe');
    await expect(d.getByTestId('search-everything-feedback')).toContainText('“builder” means CompanyA.Report.Builder.exe');
    // The words now mean that one thing: no more question.
    await d.getByTestId('search-everything-undo').click();
    await expect(input(page)).toHaveValue('builder');
    await expect(d.getByTestId('search-everything-clarify')).toBeVisible();
  });
});
