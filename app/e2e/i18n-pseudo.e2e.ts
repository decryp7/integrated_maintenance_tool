import { expect, Locator, Page, test } from '@playwright/test';
import { APP_LOAD_TIMEOUT } from './app';

// The generated pseudo-locale (Settings ▸ General ▸ Language ▸ Pseudo): accented, longer, bracketed (⟦…⟧) text.
// Smoke test of runtime translation loading, and a guard against unmarked English in the shell chrome and menus.

const PSEUDO = /⟦[^⟧]*⟧/;
/** Text that is never translated: the product name, units of time zones/formats, key names, glyphs. */
const ALLOWED = /^(Debrief|UTC|ISO|English|Ctrl|Shift|Alt|Cmd|Esc|Enter|Tab|F\d+|[A-Z])$/;

async function openPseudo(page: Page): Promise<void> {
  await page.addInitScript(() => {
    // Seed once per tab, so a reload keeps what the test chose.
    if (sessionStorage.getItem('debrief.e2e.seeded')) return;
    sessionStorage.setItem('debrief.e2e.seeded', '1');
    localStorage.clear();
    localStorage.setItem('debrief.language', JSON.stringify({ version: 1, language: 'pseudo' }));
  });
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('lang', 'en-XA');
  // The language is set before the app starts; the first rows come with the app's load, which takes as long as anywhere.
  await expect(page.locator('debrief-events-grid [role=row].row').first()).toBeVisible({ timeout: APP_LOAD_TIMEOUT });
}

/**
 * English words in the visible text and labels of `scope` that are not inside a pseudo-translated ⟦…⟧ run, so an
 * unmarked string shows up as e.g. "Open file". Data (file names, counts) must be kept out of `scope`.
 */
async function unmarkedEnglish(scope: Locator): Promise<string[]> {
  const texts = await scope.evaluateAll((els) => {
    const out: string[] = [];
    for (const el of els) {
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
      for (let n: Node | null = el; n; n = walker.nextNode()) {
        if (n instanceof Element) {
          const style = getComputedStyle(n);
          if (style.display === 'none' || style.visibility === 'hidden') continue;
          for (const a of ['aria-label', 'title', 'placeholder']) {
            const v = n.getAttribute(a);
            if (v) out.push(v);
          }
        } else if (n.parentElement?.closest('kbd, .kbd, .keys, .shortcut') === null) {
          out.push(n.textContent ?? '');
        }
      }
    }
    return out;
  });
  const words = new Set<string>();
  for (const t of texts) {
    for (const w of t.replace(/⟦[^⟧]*⟧/g, ' ').match(/[A-Za-z]{2,}/g) ?? []) if (!ALLOWED.test(w)) words.add(w);
  }
  return [...words];
}

test.describe('pseudo-locale', () => {
  test.beforeEach(async ({ page }) => openPseudo(page));

  test('the app loads in pseudo and the shell shows translated text', async ({ page }) => {
    const bar = page.locator('debrief-activity-bar');
    await expect(bar.locator('button').first()).toHaveAttribute('aria-label', PSEUDO);
    await expect(page.getByRole('menubar')).toHaveAttribute('aria-label', PSEUDO);
    await expect(page.getByRole('menubar').getByRole('menuitem').first()).toHaveText(PSEUDO);
    await expect(page.getByRole('grid').first()).toHaveAttribute('aria-label', PSEUDO);
  });

  test('no unmarked English in the activity bar, the menu bar and its menus', async ({ page }) => {
    expect(await unmarkedEnglish(page.locator('debrief-activity-bar'))).toEqual([]);
    const menubar = page.getByRole('menubar');
    expect(await unmarkedEnglish(menubar)).toEqual([]);
    const top = menubar.getByRole('menuitem');
    const count = await top.count();
    expect(count).toBeGreaterThan(3);
    for (let i = 0; i < count; i++) {
      await top.nth(i).click();
      const menu = page.getByRole('menu').last();
      await expect(menu).toBeVisible();
      // Recent files/projects and similar list data; everything else in a menu is UI text.
      expect(await unmarkedEnglish(menu.locator('[role^=menuitem]:not([data-recent]), [role=separator] + [role=presentation]')), `menu ${i}`).toEqual([]);
      await page.keyboard.press('Escape');
      await expect(page.getByRole('menu')).toHaveCount(0);
    }
  });

  test('Settings ▸ General shows pseudo text and the language choice', async ({ page }) => {
    await page.locator('debrief-activity-bar button').last().click();
    const general = page.locator('debrief-general-settings');
    await expect(general).toBeVisible();
    const language = general.getByTestId('language-setting');
    await expect(language.locator('legend')).toHaveText(PSEUDO);
    const select = language.getByTestId('language-select');
    await expect(select.locator('button')).toContainText(/\S/);
    await expect(select.locator('button')).toHaveAttribute('aria-label', PSEUDO);
    await select.locator('button').click();
    await expect(page.getByRole('option', { name: 'English' })).toBeVisible();
    await page.keyboard.press('Escape');
    expect(await unmarkedEnglish(general.locator('legend, label, .btn'))).toEqual([]);
  });

  test('choosing English asks to restart and applies after the reload', async ({ page }) => {
    await page.locator('debrief-activity-bar button').last().click();
    await page.locator('debrief-general-settings').getByTestId('language-select').locator('button').click();
    await page.getByRole('option', { name: 'English' }).click();
    const dialog = page.getByRole('dialog').last();
    await expect(dialog).toContainText(PSEUDO);
    // Later: the window keeps running in pseudo and says a restart is needed.
    await dialog.getByRole('button').filter({ hasText: PSEUDO }).first().click();
    await expect(page.locator('debrief-general-settings [role=status]')).toHaveText(PSEUDO);
    await expect(page.locator('html')).toHaveAttribute('lang', 'en-XA');
    await page.evaluate(() => localStorage.getItem('debrief.language')).then((v) => expect(JSON.parse(v ?? '{}').language).toBe('en'));
    // Restart now (the browser build reloads the page): the app comes back in English.
    await page.locator('debrief-general-settings [role=status] button').click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en-US');
    await expect(page.getByRole('menubar', { name: 'Application menu' })).toBeVisible();
  });
});

test('English stays the default and loads no translations', async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('lang', /^en/);
  await expect(page.getByRole('menubar', { name: 'Application menu' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Explorer' })).toBeVisible();
});
